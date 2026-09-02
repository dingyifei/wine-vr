//! The launch preflight — run.sh lines 6–91.
//!
//! Every `# preflight:` and `# preflight-autofix:` tag in run.sh names one
//! contract slug, and the contract's `native_gate` column says what this side
//! does with a failure:
//!
//! * `block` → [`crate::error::SabrageError::Fatal`], the `die` equivalent;
//! * `warn` → a warn row, launch continues;
//! * `autofix` → apply the slug's fix, re-check, and only then decide;
//! * `none` → not part of this side's preflight at all.
//!
//! The gates are contract data precisely so a per-side divergence
//! (`cfg.protocol.legacy-oxrsys` is `warn` in the shell and `block` here) is
//! recorded once, in `contract/pipeline.toml`, rather than discovered by
//! reading two implementations.
//!
//! Each evaluated check emits exactly one [`crate::events::StageEvent::Check`]
//! carrying its **final** outcome — for an `autofix` slug that is the outcome
//! of the re-check, preceded by the [`crate::events::StageEvent::AutoFixed`]
//! describing what changed.
//!
//! # Order is load-bearing
//!
//! [`preflight_slugs`] returns contract order, which is doctor's order, in
//! which the bottle context resolves before anything consumes it. The parity
//! harness asserts run.sh's `# preflight:` tags against this same list, so a
//! check added to one side and not the other fails a test rather than a
//! launch.
//!
//! ## …but it is *not* run.sh's order
//!
//! run.sh's own sequence is game → wine → bridge → host → bottle → overlay →
//! backend → goldberg → protocol → helper. Contract order is doctor's:
//! bottle → goldberg → game → helper → overlay → bottle-bridge → host →
//! protocol → run-only. Both sides evaluate the **same set** and abort on the
//! same conditions; only *which* die wins when several would fire at once can
//! differ. Two visible consequences, both declared in `sabrage/PARITY.md`:
//!
//! * with CrossOver absent, the shell dies on `run.wine-exec` while this side
//!   dies earlier, on `overlay.dxmt-d3d11` being unverifiable;
//! * with `oxrsys-runtime.toml` missing *and* the arm64 helper unstaged, the
//!   shell dies on the missing toml (line 56) and this side on the helper —
//!   because a missing file yields the shell's own `${ENCODER_PROC:-auto}`
//!   default, which requires the helper.
//!
//! # Skipped is never Pass
//!
//! A check may be skipped for two very different reasons, and they are not
//! interchangeable:
//!
//! * **not applicable** — the shell would not have evaluated it either
//!   (`run.wired-adb` without `--wired`, the helper pair under
//!   `encoder_process = "inproc"`). Emitted as a `Skipped` check row; never
//!   blocks.
//! * **applicable but unverifiable** — the probe could not reach a verdict
//!   (CrossOver.app missing under an `overlay.*` row, adb probing switched
//!   off under `--wired`). That is a Fatal, not a pass: launching on an
//!   unverified gate is how a black window happens.

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
/// `encoder_process = "inproc"` (run.sh lines 85–91).
const HELPER_SLUGS: [&str; 2] = ["build.helper-staged", "build.helper-arm64"];

/// The launch-preflight slugs this side evaluates, in contract order.
///
/// Exactly `contract().native_preflight()` — every check whose `native_gate`
/// is gating. Derived, never hand-written: the parity crate joins run.sh's
/// `# preflight:` tags against this, and a hand-maintained second list is how
/// the two drift.
pub fn preflight_slugs() -> Vec<&'static str> {
    contract()
        .native_preflight()
        .into_iter()
        .map(|c| c.slug.as_str())
        .collect()
}

// ── the two config facts, read once ───────────────────────────────────────────

/// `oxrsys-runtime.toml` read once, before the checks that branch on it — the
/// same two facts run.sh captures with `awk` on lines 57 and 70, resolved the
/// way the runtime resolves them ([`read_toml_facts`]).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TomlFacts {
    /// `[ -f "$TOML" ]`.
    present: bool,
    /// `$PROTOCOL` — the last value the runtime would **accept**, else the raw
    /// last assignment, and `""` when the key is absent or the file does not
    /// exist.
    protocol: String,
    /// `${ENCODER_PROC:-auto}` — already defaulted, exactly like the shell
    /// parameter expansion on line 71.
    encoder_process: String,
}

/// One key's **raw** last assignment:
/// [`crate::config::runtime_toml::effective_string`] — `Config.cpp`'s
/// `ParseConfigToml` loop, narrowed to a single key. Quote-aware `#`
/// stripping, `[table]` headers ignored, split on the first `=`, quotes
/// removed, and the **last** assignment wins, with no accepted-set filtering.
/// An unassigned key is the empty string, which is what the callers below
/// already treat as "unset".
///
/// This is the right reader for a key whose accepted set Sabrage does not
/// model, and — for the two keys it does model — the right *fallback* when no
/// occurrence at all is one the runtime would accept: that is the value the
/// die text has to quote back to the user ([`read_toml_facts`]).
fn effective_string(toml_text: &str, key: &str) -> String {
    crate::config::runtime_toml::effective_string(toml_text, key).unwrap_or_default()
}

/// run.sh lines 56–57 and 70–71, in one read of the file — but resolved the way
/// the **runtime** resolves them, not the way `awk` does.
///
/// `protocol` and `encoder_process` are two of the six keys Sabrage models, so
/// they go through [`crate::config::runtime_toml::read_lines_like_the_runtime`]:
/// `Config.cpp` assigns only inside its whitelist and its `catch` block
/// "ignore[s] malformed values and keep[s] the last valid/default setting", so
/// the value the launched runtime uses is the last assignment it would
/// **accept** — `protocol = "alvr"` followed by `protocol = "banana"` still
/// runs ALVR. Reading the last *raw* assignment instead refused a
/// configuration the runtime accepts (and could demand the arm64 helper for an
/// `inproc` runtime). The Settings screen reads the same file through the same
/// function, so the two can no longer name different backends.
///
/// Two fallbacks keep the shell's own texts intact when nothing is accepted:
///
/// * `protocol` — the raw last assignment, so `protocol='banana'` is quoted
///   back in run.sh's die, and the empty string (an absent key, an unreadable
///   file) still reads as run.sh's empty `$PROTOCOL`;
/// * `encoder_process` — the raw last assignment, which
///   [`encoder_mode`] then reports as unrecognized-treated-as-auto, and
///   `auto` when the key is absent (`${ENCODER_PROC:-auto}`).
///
/// Last-wins across `[table]` boundaries is load-bearing either way: the
/// runtime is table-blind, so a second `protocol =` further down the file is
/// the value it uses; validating the *first* one let a launch pass every ALVR
/// gate and then start the legacy backend.
///
/// One narrow DIVERGENCE from run.sh, in this side's favour and declared in
/// `sabrage/PARITY.md`: an **unquoted** value (`protocol = alvr`) is accepted
/// here and reads as the empty string through `awk -F'"'`, which then dies.
/// The runtime accepts it, so refusing the launch would be the wrong verdict.
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

/// run.sh lines 85–91's `case "$ENCODER_PROC"`: does this configuration need
/// the staged arm64 helper, and does the shell print a line about it first?
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

// ── the walk ──────────────────────────────────────────────────────────────────

/// Why a slug was not evaluated at all. Rendered as the `Skipped` check row's
/// message, so the reason reaches the UI instead of an unexplained blank.
fn not_applicable_reason(ctx: &StageCtx, slug: &str, mode: EncoderMode) -> Option<&'static str> {
    match slug {
        // run.sh line 103: the whole `--wired` block is inside
        // `if [ -n "${WINEVR_WIRED:-}" ]`.
        "run.wired-adb" if !ctx.opts.wired => Some("not --wired"),
        // run.sh line 87: `inproc` never calls `ensure_helper_staged`.
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
    // run.sh line 6, one line above the tagged preflight block. This is what
    // enforces `bottle.named` + `bottle.exists`; their registry rows are still
    // emitted below (they cannot fail once this has passed), because a GUI
    // preflight list with two silently-absent rows reads like a bug.
    require_bottle(ctx)?;

    let facts = read_toml_facts(&ctx.paths.toml_path);
    let mode = encoder_mode(&facts.encoder_process);

    let registry = crate::checks::registry();
    // The preflight checks the bottle **this stage resolved**, not one the
    // check layer re-derives from `$HOME`: `require_bottle` above has already
    // settled which bottle (and which `BS_DIR`) the launch is about, and a
    // second, independent resolution is a way for the two to disagree. In a
    // real run they are identical by construction; overriding makes that an
    // invariant rather than a coincidence — and is what lets a test point the
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
/// which sits between evaluators — cannot interrupt it. For the child-probe
/// slugs that matters: the stage holds [`crate::stages::OPERATION_LOCK`]
/// throughout, so a wedged `adb devices` used to make Stop wait for it. Those
/// run on a blocking thread raced against the launch's token; the probe's own
/// deadline is what bounds the orphaned thread afterwards.
///
/// Everything else is evaluated inline, exactly as before: a `stat` is not
/// worth a thread hop, and doctor (which has no token at all) keeps calling
/// the same evaluators directly.
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

/// run.sh line 87's `info`, verbatim.
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

/// run.sh line 15's `warn` — the one `warn`-gated row's text, which is not
/// doctor's. `pub` (A1-3), same reason as [`INPROC_NOTICE`].
pub fn game_version_warn(version: &str) -> String {
    format!("Beat Saber version '{version}' != 1.29.4 — the Meta gate may block startup")
}

/// run.sh lines 87 / 89, verbatim.
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

    // ── per-slug override: dep.goldberg ────────────────────────────────────
    //
    // run.sh line 54: `sha256_ok "$GBE_DLL" … || [ -f "$GBE_DLL" ] || die`.
    // The launch dies only when the dll is *gone*; a hash that differs from
    // the pinned build is tolerated (a user-supplied Goldberg build is a
    // legitimate setup). Doctor is stricter on purpose, so its verdict is
    // reported as-is and simply not gated when the file exists.
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

    // ── per-slug override: the two protocol rows ───────────────────────────
    //
    // Both are decided from the single `$PROTOCOL` read (run.sh line 57)
    // rather than from the evaluator's own re-read, so the two rows can never
    // disagree with each other or with `PreflightFacts`.
    if slug == "cfg.protocol.supported" || slug == "cfg.protocol.legacy-oxrsys" {
        return protocol_gate(ctx, spec, outcome, facts);
    }

    match outcome.status {
        CheckStatus::Pass | CheckStatus::Info => {
            emit_check(ctx, outcome, spec.native_gate);
            Ok(())
        }

        CheckStatus::Warn => {
            // run.sh line 15 is the only `warn`-gated row, and its text is
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

/// "Applicable but unverifiable" — the row was emitted, and the launch stops.
///
/// A `Skipped` outcome that reached a gate is **not** a pass (design-core
/// §10's S11): the probe reached no verdict, and launching on an unverified
/// gate is exactly how a black window happens. The reason the check gave is
/// carried into the die text, with its remedy appended when it has one, so the
/// user reads why it could not be checked rather than a bare slug.
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

/// run.sh lines 56–63, decided from the single `$PROTOCOL` capture.
fn protocol_gate(
    ctx: &StageCtx,
    spec: &CheckSpec,
    outcome: CheckOutcome,
    facts: &TomlFacts,
) -> Result<()> {
    let slug = spec.slug.as_str();
    let toml = ctx.paths.toml_path.display().to_string();

    // Every branch below emits this same row and then decides, so it is
    // emitted once, here: the row reports the check, the branch reports the
    // gate.
    //
    // The row is the *doctor* evaluator's verdict (`awk -F'"'`, last raw
    // assignment), and the gate below is this side's runtime-semantics fact —
    // the divergence declared in `sabrage/PARITY.md`. When the two disagree
    // and the launch proceeds anyway (an unquoted value; a valid assignment
    // shadowed by a later junk one), the row would otherwise read as a red
    // check the launch silently ignored, so it says why instead.
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
        // block). run.sh line 60 warns and launches the legacy USB path;
        // Sabrage v1 does not implement it, so it refuses rather than
        // launching something it cannot supervise. The first line is run.sh's
        // warn text verbatim; the second says what this side does instead.
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

        // Anything else: run.sh lines 61–62's two-line die, attributed to the
        // supported-set row. The legacy row is `tap … skipped` in the shell
        // and never reached here (the die above aborts first).
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
/// Three of these have no shell counterpart at all
/// (`overlay.dxmt-winemetal`, `overlay.woxr-dll`, `overlay.woxr-so`: run.sh
/// `cmp`s only `d3d11.dll`, the contract records the divergence) and reuse the
/// sentence shape of the `d3d11` die they extend.
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

        // run.sh line 54.
        "dep.goldberg" => (
            "Goldberg dll missing — ./demo.sh setup".to_string(),
            Some("./demo.sh setup".to_string()),
        ),

        // run.sh lines 10–12.
        "game.present" => (
            format!(
                "Beat Saber not found at {}\n       download 1.29.4: {}\n       \
                 (or pass --bs-dir / set WINEVR_BS_DIR)",
                ctx.bs_dir.display(),
                contract().depot_command(&ctx.bs_dir)
            ),
            None,
        ),

        // run.sh lines 32–33. `dxmt-winemetal` is the same overlay, so it
        // reuses the same sentence.
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

        // run.sh line 25.
        "bottle.woxr-dll" => (
            format!("bottle wineopenxr.dll stale/missing — {install}"),
            install_remedy,
        ),
        // run.sh line 27.
        "bottle.manifest" => (
            format!("bottle OpenXR manifest missing — {install}"),
            install_remedy,
        ),
        // run.sh line 30.
        "bottle.registry" => (
            format!("bottle ActiveRuntime registry key missing — {install}"),
            install_remedy,
        ),
        // run.sh line 21.
        "host.manifest" => (
            format!("host OpenXR registration missing — {install}"),
            install_remedy,
        ),

        // run.sh lines 17, 19, 104, 105 — `checks::run_only` already carries
        // each die string whole, because those slugs have no doctor row whose
        // prose could compete with run.sh's.
        "run.wine-exec" | "run.bridge-built" | "run.wired-adb" => {
            (outcome.message.clone(), outcome.remedy.clone())
        }

        // Not reachable through a `block` gate today; falling back on the
        // check's own words beats inventing a sentence.
        _ => (outcome.message.clone(), outcome.remedy.clone()),
    }
}

// ── the two auto-fixes ────────────────────────────────────────────────────────

/// The `autofix` gate: apply the mapped fix, re-evaluate, and only then decide.
///
/// run.sh's two auto-fixing preflights are the `cxbottle.conf` backend rewrite
/// (lines 38–52) and `ensure_helper_staged` (lines 72–91). Both are
/// **permanent** mutations, never unwound — see [`super`]'s "permanent vs
/// guarded".
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
/// `Check` + one `Fatal` this module promises.
///
/// Without this an `Err` out of [`apply_fix`] left the slug with **no** check
/// row at all (the `?` returned above every emit), so an event-only consumer
/// saw a failed `StageFinished` and an unresolved row. Three shapes:
///
/// * `Cancelled` — Stop, not a failure. Propagated untouched, exactly like the
///   walk's own checkpoint: no row, no die.
/// * `Fatal` — the fix already emitted its own `Fatal` (`ctx.fatal`, e.g.
///   `helper::restage_helper`'s "neither the staged copy nor the build output
///   is arm64"). Its text is run.sh's; only the missing `Check` is added.
/// * anything else (a raw `Io` from `write_atomic` / `copy_if_changed`) — the
///   io cause is surfaced as a stderr-shaped `Output` line and the die is
///   run.sh's post-fix text (`could not force graphics backend to dxmt in …`),
///   the same shape `actions::die_with_cause` uses.
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

/// run.sh's die text for "the auto-fix ran and the condition is still there"
/// (lines 42/46/49 and line 78).
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

// ── small ctx helper ──────────────────────────────────────────────────────────

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
mod tests {
    use super::*;
    use crate::checks::CheckStatus;
    use crate::contract::{CONTRACT_FILES, CONTRACT_GEN_REL_PATH};
    use crate::events::StageEvent;
    use crate::paths::{Bottle, Paths};
    use crate::stages::{StageCtx, StageOptions};
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    // ── the contract-derived slug list (unchanged from the frame) ───────────

    #[test]
    fn the_slug_list_is_the_contracts_gating_set_in_order() {
        let slugs = preflight_slugs();
        assert!(!slugs.is_empty());

        let expected: Vec<&str> = contract()
            .checks
            .iter()
            .filter(|c| c.native_gate.is_gating())
            .map(|c| c.slug.as_str())
            .collect();
        assert_eq!(slugs, expected, "must be contract order, unmodified");

        // No duplicates, and nothing gated `none` sneaks in.
        let unique: std::collections::BTreeSet<_> = slugs.iter().collect();
        assert_eq!(unique.len(), slugs.len());
        for slug in &slugs {
            let spec = contract().check(slug).expect("slug is in the contract");
            assert_ne!(spec.native_gate, Gate::None, "{slug} is doctor-only");
        }

        // The run-only slugs exist only here — they have no doctor row at all.
        assert!(slugs.contains(&"run.wine-exec"));
        assert!(slugs.contains(&"run.bridge-built"));
    }

    /// Every `autofix`-gated slug must have an arm in [`apply_fix`], and every
    /// `block`-gated slug an entry in [`block_die`]'s table. The `_ =>` arms
    /// make both total at compile time, so this asserts the *set* instead.
    #[test]
    fn every_gated_slug_is_accounted_for() {
        for slug in preflight_slugs() {
            let spec = contract().check(slug).unwrap();
            match spec.native_gate {
                Gate::Autofix => {
                    assert!(
                        slug == "bottle.gfx-dxmt" || HELPER_SLUGS.contains(&slug),
                        "{slug} is gated autofix but apply_fix has no arm"
                    );
                    // `apply_fix` dispatches off the contract's own `fix` id,
                    // so an autofix slug that names no applicable fix would
                    // die at launch instead of fixing anything.
                    assert!(
                        spec.fix
                            .as_deref()
                            .and_then(FixAction::from_contract_id)
                            .is_some(),
                        "{slug} is gated autofix but names no applicable fix"
                    );
                }
                Gate::Warn => assert_eq!(
                    slug, "game.version",
                    "a second warn-gated slug needs its run.sh text in `gate`"
                ),
                Gate::Block | Gate::None => {}
            }
        }
    }

    // ── the config reader ───────────────────────────────────────────────────

    /// The same file, read by this module and by the Settings view, must name
    /// the same backend — the bug was preflight validating `[streaming]`'s
    /// `alvr` while the runtime obeyed a later `oxrsys`.
    #[test]
    fn the_preflight_facts_agree_with_the_settings_view_on_a_shadowed_key() {
        let dir = scratch("shadowed-agree");
        let toml = dir.join("oxrsys-runtime.toml");
        std::fs::write(
            &toml,
            b"[streaming]\nprotocol = \"alvr\"\nencoder_process = \"inproc\"\n\n              [tweaks]\nprotocol = \"oxrsys\"\nencoder_process = \"native\"\n",
        )
        .unwrap();

        let facts = read_toml_facts(&toml);
        let view = crate::config::runtime_toml::read(&toml);
        assert_eq!(
            facts.protocol,
            view.values.protocol.unwrap().as_str(),
            "preflight and the Settings view must read one value"
        );
        assert_eq!(
            facts.encoder_process,
            view.values.encoder_process.unwrap().as_str()
        );

        // A3b-1/A7-1: a *valid* assignment shadowed by a later INVALID one.
        // Config.cpp assigns only inside its whitelist, so the runtime keeps
        // alvr/inproc — the raw-last reading (`banana`/`garbage`) refused a
        // configuration the runtime accepts and demanded the arm64 helper for
        // an in-process encode.
        std::fs::write(
            &toml,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n[tweaks]\nprotocol = \"banana\"\nencoder_process = \"garbage\"\n",
        )
        .unwrap();
        let facts = read_toml_facts(&toml);
        let view = crate::config::runtime_toml::read(&toml);
        assert_eq!(facts.protocol, "alvr");
        assert_eq!(facts.encoder_process, "inproc");
        assert_eq!(facts.protocol, view.values.protocol.unwrap().as_str());
        assert_eq!(
            facts.encoder_process,
            view.values.encoder_process.unwrap().as_str()
        );

        // …but when NO occurrence is one the runtime would accept, the raw
        // last assignment is still what the die text and the unrecognized-
        // encoder warn quote back.
        std::fs::write(
            &toml,
            b"protocol = \"banana\"\nencoder_process = \"garbage\"\n",
        )
        .unwrap();
        let facts = read_toml_facts(&toml);
        assert_eq!(facts.protocol, "banana");
        assert_eq!(facts.encoder_process, "garbage");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn encoder_process_defaults_to_auto_exactly_like_the_shell() {
        let dir = scratch("facts");
        let toml = dir.join("oxrsys-runtime.toml");

        // Missing file: `awk` on a nonexistent path captures nothing.
        let facts = read_toml_facts(&toml);
        assert!(!facts.present);
        assert_eq!(facts.protocol, "");
        assert_eq!(facts.encoder_process, "auto", "${{ENCODER_PROC:-auto}}");

        // Present, key absent.
        std::fs::write(&toml, "protocol = \"alvr\"\n").unwrap();
        let facts = read_toml_facts(&toml);
        assert!(facts.present);
        assert_eq!(facts.protocol, "alvr");
        assert_eq!(facts.encoder_process, "auto");

        // Present, key set.
        std::fs::write(&toml, "protocol = \"alvr\"\nencoder_process = \"inproc\"\n").unwrap();
        assert_eq!(read_toml_facts(&toml).encoder_process, "inproc");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── fixtures ────────────────────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "sabrage-preflight-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(p: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// The checkout this test binary was compiled from — three levels above
    /// the crate manifest (`sabrage/crates/sabrage-core`), the same recipe
    /// `checks::meta`'s and `util`'s own tests use.
    fn checkout_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("repo root resolves")
    }

    /// Copy the checkout's `contract/` and its generated shell mirror into a
    /// scratch root, so the walk's **first** slug — `meta.contract-sync`,
    /// `block`-gated on this side — passes there.
    ///
    /// That evaluator reconciles three hashes: the scratch root's own
    /// `contract/`, the `# contract-sha256:` header of its
    /// `scripts/demo/contract.gen.sh`, and the contract THIS binary was
    /// compiled with. Copying the live files satisfies all three at once
    /// (the compiled-in copy came from the same checkout), which is what lets
    /// every test below reach the row it is actually about instead of dying
    /// on row zero.
    fn seed_contract(root: &Path) {
        let src = checkout_root();
        for rel in CONTRACT_FILES
            .iter()
            .copied()
            .chain([CONTRACT_GEN_REL_PATH])
        {
            let dst = root.join(rel);
            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
            std::fs::copy(src.join(rel), &dst)
                .unwrap_or_else(|e| panic!("seeding {rel} into the scratch root: {e}"));
        }
    }

    fn write_exec(p: &Path, bytes: &[u8]) {
        write(p, bytes);
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    struct Fixture {
        root: PathBuf,
        ctx: StageCtx,
        events: Arc<StdMutex<Vec<StageEvent>>>,
    }

    impl Fixture {
        fn events(&self) -> Vec<StageEvent> {
            self.events.lock().unwrap().clone()
        }

        fn checks(&self) -> Vec<(String, CheckStatus)> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    StageEvent::Check { outcome, .. } => Some((outcome.slug, outcome.status)),
                    _ => None,
                })
                .collect()
        }

        fn check(&self, slug: &str) -> Option<CheckOutcome> {
            self.events().into_iter().find_map(|e| match e {
                StageEvent::Check { outcome, .. } if outcome.slug == slug => Some(outcome),
                _ => None,
            })
        }

        fn lines(&self) -> Vec<String> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    StageEvent::Line { text, .. } => Some(text),
                    _ => None,
                })
                .collect()
        }

        fn auto_fixed(&self) -> Vec<FixAction> {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    StageEvent::AutoFixed { fix, .. } => Some(fix),
                    _ => None,
                })
                .collect()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    /// A fully synthetic pipeline root: fixture bottle, fixture OXRSys support
    /// dir, fixture CrossOver tree. Nothing under the real `$HOME`,
    /// `/usr/local`, or `CrossOver.app` is read or written, and the executor
    /// is [`crate::executor::DryRunExecutor`] unless `dry_run` is false.
    fn fixture(tag: &str, dry_run: bool) -> Fixture {
        let root = scratch(tag);
        // Row zero of the walk checks the checkout itself, so the scratch root
        // has to look like a real (self-consistent) checkout.
        seed_contract(&root);
        let prefix = root.join("bottle");
        let cx = root.join("CrossOver");

        let mut paths = Paths::new(&root);
        paths.oxr_appsup = root.join("OXRSys");
        paths.toml_path = paths.oxr_appsup.join("oxrsys-runtime.toml");
        paths.sabrage_appsup = root.join("Sabrage");
        paths.host_xr_json = root.join("host/active_runtime.x86_64.json");
        paths.cx_app = Some(cx.join("CrossOver.app"));
        paths.cx = Some(cx.clone());
        paths.wine = Some(cx.join("bin/wine"));
        paths.wineserver = Some(cx.join("bin/wineserver"));
        paths.adb = None;

        let opts = StageOptions {
            bottle_name: Some("FixtureBottle".to_string()),
            // Never let `BS_DIR` fall back to the real bottles root: the
            // default derives from `$HOME`, and these tests write a fake
            // `Beat Saber.exe` into it.
            bs_dir_override: Some(root.join("BeatSaber")),
            dry_run,
            ..StageOptions::default()
        };

        let events: Arc<StdMutex<Vec<StageEvent>>> = Arc::new(StdMutex::new(Vec::new()));
        let seen = events.clone();
        let sink: crate::stages::EventSink = Arc::new(move |e| seen.lock().unwrap().push(e));

        let mut ctx = StageCtx::new(paths, opts, sink, CancellationToken::new());
        ctx.bottle = Some(Bottle {
            name: "FixtureBottle".to_string(),
            sys32: prefix.join("drive_c/windows/system32"),
            prefix: prefix.clone(),
        });

        // Belt and braces: every path this preflight can write through must
        // stay inside the scratch root. `BS_DIR` in particular defaults to a
        // path under the machine's REAL bottles directory.
        assert!(
            ctx.bs_dir.starts_with(&root),
            "fixture BS_DIR escaped the scratch root: {}",
            ctx.bs_dir.display()
        );
        assert!(ctx
            .bottle
            .as_ref()
            .is_some_and(|b| b.prefix.starts_with(&root)));

        Fixture { root, ctx, events }
    }

    /// Make every `block`-gated slug before `cfg.protocol.*` pass, so a test
    /// can drive the preflight all the way down to the row it cares about.
    fn make_everything_pass(f: &Fixture) {
        let p = &f.ctx.paths;
        let b = f.ctx.bottle.clone().unwrap();

        // bottle.gfx-dxmt (autofix) — already current, so no fix runs.
        write(
            &b.conf_path(),
            b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        );
        // dep.goldberg — present (hash will not match the pin; run tolerates).
        write(&p.gbe_dll, b"not the pinned build");
        // game.present / game.version
        write(&f.ctx.bs_dir.join("Beat Saber.exe"), b"MZ");
        write(&f.ctx.bs_dir.join("BeatSaberVersion.txt"), b"1.29.4\n");
        // build.helper-* — staged copy is this test binary (thin arm64 here).
        let helper = std::env::current_exe().unwrap();
        std::fs::create_dir_all(p.oxr_helper_staged.parent().unwrap()).unwrap();
        std::fs::copy(&helper, &p.oxr_helper_staged).ok();
        // overlay.* — src == dst for all four.
        for (src, dst) in [
            (
                p.dxmt_art.join("x86_64-windows/d3d11.dll"),
                p.cx_dxmt("x86_64-windows/d3d11.dll").unwrap(),
            ),
            (
                p.dxmt_art.join("x86_64-unix/winemetal.so"),
                p.cx_dxmt("x86_64-unix/winemetal.so").unwrap(),
            ),
            (
                p.woxr_dll.clone(),
                p.cx_wine_lib("x86_64-windows/wineopenxr.dll").unwrap(),
            ),
            (
                p.woxr_so.clone(),
                p.cx_wine_lib("x86_64-unix/wineopenxr.so").unwrap(),
            ),
        ] {
            write(&src, b"overlay-bytes");
            write(&dst, b"overlay-bytes");
        }
        // bottle-bridge
        write(&b.sys32.join("wineopenxr.dll"), b"overlay-bytes");
        write(&b.openxr_manifest(), b"{}");
        write(
            &b.system_reg(),
            b"\"ActiveRuntime\"=\"C:\\\\openxr\\\\wineopenxr64.json\"\n",
        );
        // host.manifest — must parse and point at an existing dylib.
        write(&p.oxr_dylib, b"dylib");
        write(
            &p.host_xr_json,
            format!(
                "{{\"file_format_version\":\"1.0.0\",\"runtime\":{{\"name\":\"oxrsys\",\
                 \"library_path\":\"{}\"}}}}\n",
                p.oxr_dylib.display()
            )
            .as_bytes(),
        );
        // run.wine-exec / run.bridge-built
        write_exec(p.wine.as_ref().unwrap(), b"#!/bin/sh\n");
        write(&p.woxr_dll, b"overlay-bytes");
        // cfg.protocol.*
        write(&p.toml_path, b"protocol = \"alvr\"\n");
    }

    // ── applicability ───────────────────────────────────────────────────────

    #[test]
    fn applicability_table() {
        let mut f = fixture("applicability", true);

        // --wired off: run.wired-adb is not applicable.
        assert_eq!(
            not_applicable_reason(&f.ctx, "run.wired-adb", EncoderMode::HelperRequired),
            Some("not --wired")
        );
        f.ctx.opts.wired = true;
        assert_eq!(
            not_applicable_reason(&f.ctx, "run.wired-adb", EncoderMode::HelperRequired),
            None
        );

        // inproc: both helper slugs are not applicable; everything else is.
        for slug in HELPER_SLUGS {
            assert_eq!(
                not_applicable_reason(&f.ctx, slug, EncoderMode::Inproc),
                Some("encoder_process=inproc — the native helper is disabled")
            );
            assert_eq!(
                not_applicable_reason(&f.ctx, slug, EncoderMode::HelperRequired),
                None
            );
            assert_eq!(
                not_applicable_reason(&f.ctx, slug, EncoderMode::UnrecognizedTreatedAsAuto),
                None,
                "an unrecognized value still requires the helper"
            );
        }
        assert_eq!(
            not_applicable_reason(&f.ctx, "game.present", EncoderMode::Inproc),
            None
        );
    }

    // ── require_bottle comes first ──────────────────────────────────────────

    /// Stop during the preflight aborts it without a die row — the walk is
    /// read-only, so there is nothing to unwind.
    #[tokio::test]
    async fn a_cancelled_token_stops_the_walk() {
        let f = fixture("cancelled", true);
        make_everything_pass(&f);
        f.ctx.cancel.cancel();

        let err = run(&f.ctx).await.unwrap_err();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert!(f.checks().is_empty(), "aborted before the first row");
    }

    #[tokio::test]
    async fn require_bottle_dies_before_any_check_row() {
        let mut f = fixture("no-bottle", true);
        f.ctx.opts.bottle_name = None;
        f.ctx.bottle = None;

        let err = run(&f.ctx).await.unwrap_err();
        assert!(
            err.to_string()
                .starts_with("CrossOver bottle name required"),
            "{err}"
        );
        assert!(f.checks().is_empty(), "no check row before require_bottle");
    }

    // ── the happy path ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_clean_machine_walks_every_slug_in_contract_order() {
        let f = fixture("clean", true);
        make_everything_pass(&f);

        let facts = run(&f.ctx).await.expect("preflight passes");
        assert_eq!(facts.protocol, "alvr");
        assert_eq!(facts.encoder_process, "auto");

        let seen: Vec<String> = f.checks().into_iter().map(|(s, _)| s).collect();
        let want: Vec<String> = preflight_slugs().iter().map(|s| s.to_string()).collect();
        assert_eq!(seen, want, "one Check per slug, in contract order");
        // Derived above, but named here because it is the newest gate and the
        // one the fixture has to seed a whole scratch checkout to satisfy.
        assert_eq!(
            seen.first().map(String::as_str),
            Some("meta.contract-sync"),
            "the contract tripwire is row zero of the walk: {seen:?}"
        );

        // run.wired-adb is the only skipped row on a non-wired clean machine.
        for (slug, status) in f.checks() {
            if slug == "run.wired-adb" {
                assert_eq!(status, CheckStatus::Skipped);
            } else {
                assert!(
                    matches!(status, CheckStatus::Pass | CheckStatus::Warn),
                    "{slug} = {status:?}"
                );
            }
        }
        assert!(f.auto_fixed().is_empty(), "nothing needed fixing");
        assert!(
            !f.lines().iter().any(|l| l.contains("unrecognized")),
            "encoder_process=auto is a recognized value — no unrecognized-encoder warn: {:?}",
            f.lines()
        );
    }

    // ── the contract tripwire ───────────────────────────────────────────────

    /// `meta.contract-sync` is `native_gate = "block"`: a checkout whose
    /// `contract.gen.sh` header no longer matches its `contract/` refuses to
    /// launch, on the contract's very first slug, before the preflight has
    /// probed — or auto-fixed — anything else.
    ///
    /// The slug has no arm in [`block_die`], so the die text is the
    /// evaluator's own message and remedy, through the fallback arm.
    #[tokio::test]
    async fn a_stale_contract_gen_header_blocks_the_launch_on_the_first_slug() {
        let f = fixture("contract-stale", true);
        make_everything_pass(&f);
        // The shape of a real header, a hash of nothing: the seeded checkout
        // is now internally inconsistent, exactly as it would be after a
        // `contract/` edit without `scripts/dev/parity.sh --regen`.
        write(
            &f.root.join(CONTRACT_GEN_REL_PATH),
            b"# contract-sha256: \
              0000000000000000000000000000000000000000000000000000000000000000\n",
        );

        let err = run(&f.ctx).await.unwrap_err();
        let SabrageError::Fatal { message, remedy } = &err else {
            panic!("a block gate must be Fatal, got {err:?}")
        };
        assert_eq!(
            message,
            "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without \
             regen, or the generated file was hand-edited)"
        );
        assert_eq!(remedy.as_deref(), Some("scripts/dev/parity.sh --regen"));

        // Row zero, and nothing after it: the walk stops on the first block.
        assert_eq!(
            f.checks(),
            vec![("meta.contract-sync".to_string(), CheckStatus::Fail)]
        );
        assert!(
            f.auto_fixed().is_empty(),
            "nothing runs behind a stale contract"
        );
        // The Fatal carries the same remedy the check row does, so the GUI
        // offers `--regen` rather than a bare slug.
        let fatals: Vec<Option<String>> = f
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::Fatal { remedy, .. } => Some(remedy),
                _ => None,
            })
            .collect();
        assert_eq!(
            fatals,
            vec![Some("scripts/dev/parity.sh --regen".to_string())]
        );
    }

    // ── dep.goldberg tolerance ──────────────────────────────────────────────

    #[tokio::test]
    async fn goldberg_hash_mismatch_does_not_block_the_launch() {
        let f = fixture("gbe-warn", true);
        make_everything_pass(&f);
        // gbe_dll is deliberately not the pinned build.
        run(&f.ctx).await.expect("a hash mismatch must not block");

        let o = f.check("dep.goldberg").expect("row emitted");
        assert_eq!(o.status, CheckStatus::Warn);
        assert!(
            o.detail.unwrap().contains("run tolerates a hash mismatch"),
            "the tolerance must be explained on the row"
        );
    }

    #[tokio::test]
    async fn a_missing_goldberg_dll_dies_with_run_shs_text() {
        let f = fixture("gbe-missing", true);
        make_everything_pass(&f);
        std::fs::remove_file(&f.ctx.paths.gbe_dll).unwrap();

        let err = run(&f.ctx).await.unwrap_err();
        assert_eq!(err.to_string(), "Goldberg dll missing — ./demo.sh setup");
        assert_eq!(f.check("dep.goldberg").unwrap().status, CheckStatus::Fail);
    }

    // ── game.version: the one warn gate ─────────────────────────────────────

    #[tokio::test]
    async fn a_wrong_game_version_warns_with_run_shs_text_and_continues() {
        let f = fixture("gamever", true);
        make_everything_pass(&f);
        write(&f.ctx.bs_dir.join("BeatSaberVersion.txt"), b"1.34.2\n");

        run(&f.ctx).await.expect("a version warn never blocks");
        assert!(
            f.lines()
                .iter()
                .any(|l| l
                    == "Beat Saber version '1.34.2' != 1.29.4 — the Meta gate may block startup"),
            "{:?}",
            f.lines()
        );
    }

    #[tokio::test]
    async fn a_missing_game_dies_with_the_three_line_die() {
        let f = fixture("game-missing", true);
        make_everything_pass(&f);
        std::fs::remove_file(f.ctx.bs_dir.join("Beat Saber.exe")).unwrap();

        let err = run(&f.ctx).await.unwrap_err();
        let want = format!(
            "Beat Saber not found at {}\n       download 1.29.4: {}\n       \
             (or pass --bs-dir / set WINEVR_BS_DIR)",
            f.ctx.bs_dir.display(),
            contract().depot_command(&f.ctx.bs_dir)
        );
        assert_eq!(err.to_string(), want);
    }

    // ── protocol branches ───────────────────────────────────────────────────

    #[tokio::test]
    async fn a_missing_toml_dies_with_the_setup_remedy() {
        let f = fixture("toml-missing", true);
        make_everything_pass(&f);
        std::fs::remove_file(&f.ctx.paths.toml_path).unwrap();

        let err = run(&f.ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{} missing — ./demo.sh setup",
                f.ctx.paths.toml_path.display()
            )
        );
    }

    #[tokio::test]
    async fn protocol_oxrsys_blocks_natively_with_both_lines() {
        let f = fixture("proto-legacy", true);
        make_everything_pass(&f);
        write(&f.ctx.paths.toml_path, b"protocol = \"oxrsys\"\n");

        let err = run(&f.ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "protocol=oxrsys (legacy USB path) — the demo path is alvr\n       \
             Sabrage does not launch the legacy protocol — use ./demo.sh run --bottle FixtureBottle"
        );
        // The supported-set row passed; the legacy row is the one that blocked.
        assert_eq!(
            f.check("cfg.protocol.supported").unwrap().status,
            CheckStatus::Pass
        );
        assert_eq!(
            f.check("cfg.protocol.legacy-oxrsys").unwrap().status,
            CheckStatus::Fail
        );
    }

    #[tokio::test]
    async fn an_unknown_protocol_dies_with_run_shs_two_line_text() {
        let f = fixture("proto-garbage", true);
        make_everything_pass(&f);
        write(&f.ctx.paths.toml_path, b"protocol = \"banana\"\n");

        let err = run(&f.ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "oxrsys-runtime.toml protocol='banana' is not valid for the demo\n       \
                 set protocol = \"alvr\" in {} (or delete the file and re-run ./demo.sh setup)",
                f.ctx.paths.toml_path.display()
            )
        );
        // The legacy row is never reached (the supported-set die aborts first).
        assert!(f.check("cfg.protocol.legacy-oxrsys").is_none());
    }

    // ── encoder_process branches ────────────────────────────────────────────

    #[tokio::test]
    async fn inproc_prints_the_info_row_once_and_skips_both_helper_slugs() {
        let f = fixture("inproc", true);
        make_everything_pass(&f);
        std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).ok();
        write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n",
        );

        let facts = run(&f.ctx).await.expect("inproc needs no helper");
        assert_eq!(facts.encoder_process, "inproc");

        let notice = "encoder_process=inproc — in-process x86_64 encode (native helper disabled)";
        assert_eq!(
            f.lines().iter().filter(|l| *l == notice).count(),
            1,
            "printed exactly once"
        );
        for slug in HELPER_SLUGS {
            let o = f.check(slug).expect("row still emitted");
            assert_eq!(o.status, CheckStatus::Skipped);
            assert_eq!(
                o.message,
                "encoder_process=inproc — the native helper is disabled"
            );
        }
        assert!(f.auto_fixed().is_empty(), "no restage under inproc");
    }

    #[tokio::test]
    async fn an_unrecognized_encoder_process_warns_once_and_still_requires_the_helper() {
        let f = fixture("enc-unknown", true);
        make_everything_pass(&f);
        write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"banana\"\n",
        );

        let facts = run(&f.ctx).await.expect("the staged helper is fine");
        assert_eq!(facts.encoder_process, "banana");

        let notice = "oxrsys-runtime.toml encoder_process='banana' unrecognized — the runtime \
                      treats unknown values as auto";
        assert_eq!(f.lines().iter().filter(|l| *l == notice).count(), 1);
        assert_eq!(
            f.check("build.helper-staged").unwrap().status,
            CheckStatus::Pass,
            "the helper pair still applies"
        );
    }

    // ── Skipped while applicable is never a pass ────────────────────────────

    #[tokio::test]
    async fn an_unverifiable_applicable_check_is_fatal_not_a_pass() {
        let f = fixture("unverifiable", true);
        make_everything_pass(&f);
        // No CrossOver.app at all: every `overlay.*` row reports Skipped
        // ("CrossOver.app not found"), which is applicable-but-unverifiable.
        let mut ctx = f.ctx.clone();
        ctx.paths.cx = None;
        ctx.paths.cx_app = None;
        ctx.paths.wine = None;
        ctx.paths.wineserver = None;

        let err = run(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "cannot verify overlay.dxmt-d3d11: CrossOver.app not found"
        );
        assert_eq!(
            f.check("overlay.dxmt-d3d11").unwrap().status,
            CheckStatus::Skipped,
            "the row is still reported before the die"
        );
    }

    #[tokio::test]
    async fn wired_without_adb_dies_with_run_shs_text() {
        let f = fixture("wired-noadb", true);
        make_everything_pass(&f);
        let mut ctx = f.ctx.clone();
        ctx.opts.wired = true;
        ctx.paths.adb = None;

        let err = run(&ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "--wired needs adb (Android platform-tools) on PATH or under ~/Library/Android/sdk"
        );
    }

    // ── the autofix path ────────────────────────────────────────────────────

    /// The real thing: a `cxbottle.conf` that says `auto`, a real (non-dry)
    /// executor over a fixture bottle. The fix must run, the re-check must
    /// pass, and exactly one `AutoFixed` must be emitted — followed by the
    /// final (passing) `Check`.
    #[tokio::test]
    async fn a_failing_backend_row_is_fixed_rechecked_and_reported_once() {
        let f = fixture("autofix-backend", false);
        make_everything_pass(&f);
        let conf = f.ctx.bottle.clone().unwrap().conf_path();
        write(
            &conf,
            b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        );

        run(&f.ctx).await.expect("the auto-fix resolves it");

        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n"
        );
        assert_eq!(
            f.auto_fixed(),
            vec![FixAction::SetGraphicsBackend],
            "exactly one AutoFixed, for the one row that needed it"
        );
        let o = f.check("bottle.gfx-dxmt").unwrap();
        assert_eq!(o.status, CheckStatus::Pass, "the row reports the RE-check");
        assert_eq!(
            f.checks()
                .iter()
                .filter(|(s, _)| s == "bottle.gfx-dxmt")
                .count(),
            1,
            "one Check per slug, never the pre-fix one as well"
        );
    }

    /// A dry run plans the write and never performs it, so the re-check
    /// necessarily still fails. Reporting that as "the fix failed" would be a
    /// lie about a run that deliberately touched nothing.
    #[tokio::test]
    async fn a_dry_run_reports_the_planned_fix_instead_of_a_failed_recheck() {
        let f = fixture("autofix-dry", true);
        make_everything_pass(&f);
        let conf = f.ctx.bottle.clone().unwrap().conf_path();
        write(&conf, b"\"CX_GRAPHICS_BACKEND\" = \"auto\"\n");

        run(&f.ctx)
            .await
            .expect("a dry run must not die on its own plan");

        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
            "a dry run never writes"
        );
        assert_eq!(f.auto_fixed(), vec![FixAction::SetGraphicsBackend]);
        let o = f.check("bottle.gfx-dxmt").unwrap();
        assert_eq!(o.status, CheckStatus::Info);
        assert!(
            o.message.ends_with("— auto-fix planned (dry run)"),
            "{}",
            o.message
        );
    }

    /// Neither a staged nor a built arm64 helper: `restage_helper` itself dies
    /// with run.sh's two-line `ensure_helper_staged` text, raising exactly one
    /// `Fatal`, and the slug's Check row is emitted alongside it.
    #[tokio::test]
    async fn an_unfixable_helper_dies_once_with_run_shs_ensure_helper_text_and_its_check_row() {
        let f = fixture("helper-unfixable", false);
        make_everything_pass(&f);
        std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();

        let err = run(&f.ctx).await.unwrap_err();
        let want = format!(
            "encoder_process=auto needs the arm64 helper, but neither the staged copy\n       \
             ({}) nor the build output ({}) is an arm64 executable — ./demo.sh build",
            f.ctx.paths.oxr_helper_staged.display(),
            f.ctx.paths.oxr_helper_built.display()
        );
        assert_eq!(err.to_string(), want);
        let rows = f.checks();
        assert_eq!(
            rows.iter()
                .filter(|(s, _)| s == "build.helper-staged")
                .count(),
            1,
            "the slug's row must still be emitted: {rows:?}"
        );
        assert_eq!(
            f.events()
                .iter()
                .filter(|e| matches!(e, StageEvent::Fatal { .. }))
                .count(),
            1,
            "the fix's own Fatal, not a second one"
        );
    }

    /// A2/A7-2 regression: `[streaming] protocol = "alvr"` shadowed by a later
    /// `protocol = "oxrsys"` used to pass every ALVR gate and then launch the
    /// legacy backend. The last assignment is the one the runtime obeys, so it
    /// is the one the preflight must judge.
    #[tokio::test]
    async fn a_shadowed_protocol_is_judged_on_the_value_the_runtime_will_use() {
        let f = fixture("proto-shadowed", true);
        make_everything_pass(&f);
        write(
            &f.ctx.paths.toml_path,
            b"[streaming]\nprotocol = \"alvr\"\n\n[tweaks]\nprotocol = \"oxrsys\"\n",
        );

        let err = run(&f.ctx).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "protocol=oxrsys (legacy USB path) — the demo path is alvr\n       \
             Sabrage does not launch the legacy protocol — use ./demo.sh run --bottle FixtureBottle"
        );
    }

    /// The repository's own "every key assigned twice, the second one junk"
    /// fixture, driven through the whole preflight: what the launch judges and
    /// what the Settings screen shows must be the same six values, and none of
    /// them the junk. (`oxrsys-runtime.shadowed-invalid-last.toml` is the file
    /// `config::runtime_toml`'s tests pin the reader against.)
    #[tokio::test]
    async fn the_shadowed_invalid_last_fixture_launches_on_its_valid_values() {
        let f = fixture("proto-fixture-invalid-tail", true);
        make_everything_pass(&f);
        let fixture_toml = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/phase4/oxrsys-runtime.shadowed-invalid-last.toml");
        write(
            &f.ctx.paths.toml_path,
            &std::fs::read(&fixture_toml).expect("the phase4 fixture reads"),
        );

        let facts = run(&f.ctx).await.expect("the runtime accepts this file");
        let view = crate::config::runtime_toml::read(&f.ctx.paths.toml_path);
        assert_eq!(facts.protocol, "alvr");
        assert_eq!(facts.encoder_process, "native");
        assert_eq!(facts.protocol, view.values.protocol.unwrap().as_str());
        assert_eq!(
            facts.encoder_process,
            view.values.encoder_process.unwrap().as_str()
        );
        // `native` still needs the staged arm64 helper, and it is there.
        for slug in HELPER_SLUGS {
            assert_eq!(f.check(slug).unwrap().status, CheckStatus::Pass, "{slug}");
        }
        assert!(
            !f.lines().iter().any(|l| l.contains("unrecognized")),
            "encoder_process=native is a recognized value — no unrecognized-encoder warn: {:?}",
            f.lines()
        );
    }

    /// A7-1/A3b-1: a valid assignment shadowed by a later INVALID one is the
    /// mirror image of the test above — the runtime keeps the *valid* value,
    /// so the launch must too. The raw-last reading died on
    /// `protocol='banana'` and demanded the arm64 helper for a runtime that
    /// encodes in-process.
    #[tokio::test]
    async fn a_trailing_invalid_assignment_leaves_the_previous_valid_one_in_force() {
        let f = fixture("proto-invalid-tail", true);
        make_everything_pass(&f);
        // No staged helper at all: if the encoder fact regressed to `garbage`
        // (unrecognized ⇒ treated as auto) this would die on the helper.
        std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();
        write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n[tweaks]\nprotocol = \"banana\"\nencoder_process = \"garbage\"\n",
        );

        let facts = run(&f.ctx).await.expect("the runtime accepts this file");
        assert_eq!(facts.protocol, "alvr");
        assert_eq!(facts.encoder_process, "inproc");

        for slug in HELPER_SLUGS {
            assert_eq!(
                f.check(slug).unwrap().status,
                CheckStatus::Skipped,
                "{slug} must be skipped for an inproc runtime"
            );
        }
        assert!(
            !f.lines().iter().any(|l| l.contains("unrecognized")),
            "no unrecognized-encoder warn for a value the runtime ignores: {:?}",
            f.lines()
        );
        // The doctor evaluator still reports its own `awk` verdict (declared
        // divergence, PARITY.md) — but the row says why the launch went on.
        let row = f.check("cfg.protocol.supported").unwrap();
        assert_eq!(row.status, CheckStatus::Fail);
        assert!(
            row.detail
                .unwrap()
                .contains("the last value it would accept is protocol='alvr'"),
            "a red row the launch ignores must explain itself"
        );
    }

    /// A7-4: `run.wired-adb` shells out to `adb devices`. A wedged adb used to
    /// block inside the synchronous evaluator, so Stop could not land until it
    /// returned — with the operation lock held throughout.
    #[tokio::test]
    async fn a_cancel_during_the_wired_adb_probe_stops_the_walk_promptly() {
        let f = fixture("wired-probe-cancel", true);
        make_everything_pass(&f);
        let mut ctx = f.ctx.clone();
        ctx.opts.wired = true;
        // An `adb` that marks that it started and then never answers.
        let adb = f.root.join("platform-tools/adb");
        // `sleep 5`, not a longer one: the abandoned blocking thread outlives
        // the assertion and the test runtime waits for it on shutdown.
        write_exec(
            &adb,
            b"#!/bin/sh\n: > \"$(dirname \"$0\")/probing\"\nsleep 5\n",
        );
        ctx.paths.adb = Some(adb);

        let cancel = ctx.cancel.clone();
        let marker = f.root.join("platform-tools/probing");
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !marker.exists() && std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            cancel.cancel();
        });

        let started = std::time::Instant::now();
        let err = run(&ctx).await.unwrap_err();
        let waited = started.elapsed();
        assert!(matches!(err, SabrageError::Cancelled), "{err:?}");
        assert!(
            waited < std::time::Duration::from_secs(3),
            "Stop must not wait out the probe, waited {waited:?}"
        );
    }

    /// The same shadowing for `encoder_process`: a first `inproc` must not skip
    /// the helper pair when a later `native` is what the runtime will use.
    #[tokio::test]
    async fn a_shadowed_encoder_process_still_requires_the_helper() {
        let f = fixture("enc-shadowed", true);
        make_everything_pass(&f);
        std::fs::remove_file(&f.ctx.paths.oxr_helper_staged).unwrap();
        write(
            &f.ctx.paths.toml_path,
            b"protocol = \"alvr\"\nencoder_process = \"inproc\"\n\n              [tweaks]\nencoder_process = \"native\"\n",
        );

        // Dies on the missing helper — under the old first-match reading both
        // helper rows would have been skipped and the launch would have gone
        // ahead with no arm64 helper at all.
        let err = run(&f.ctx).await.unwrap_err();
        // (The value quoted in the die text comes from `fixes::helper`'s own
        // reader, which is still first-match — a sibling fix, cross-area.)
        assert!(err.to_string().contains("needs the arm64 helper"), "{err}");
        assert!(
            !f.lines()
                .iter()
                .any(|l| l.contains("encoder_process=inproc")),
            "the shadowed inproc must not print the in-process notice: {:?}",
            f.lines()
        );
    }

    // ── a failing auto-fix still reports its row ────────────────────────────

    /// A7-6: an `Err` out of the fix used to return through `?` before the
    /// slug's `Check` was emitted — an event-only consumer saw a failed stage
    /// with the row left hanging. One Check, one Fatal, and the io cause.
    #[tokio::test]
    async fn a_backend_autofix_that_cannot_write_still_emits_its_check_and_dies_run_shs_way() {
        let f = fixture("autofix-unwritable", false);
        make_everything_pass(&f);
        let b = f.ctx.bottle.clone().unwrap();
        let conf = b.conf_path();
        write(
            &conf,
            b"\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        );
        // Read-only directory: the atomic write cannot create its temp file.
        let dir = conf.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let err = run(&f.ctx).await.unwrap_err();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(
            err.to_string(),
            format!(
                "could not force graphics backend to dxmt in {}",
                conf.display()
            ),
            "run.sh's post-fix die text, not a raw io error"
        );
        // Exactly one Check for the slug, carrying the failure and its cause.
        let rows: Vec<CheckOutcome> = f
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::Check { outcome, .. } if outcome.slug == "bottle.gfx-dxmt" => {
                    Some(outcome)
                }
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].status, CheckStatus::Fail);
        assert!(rows[0].detail.is_some(), "the io cause belongs on the row");
        // One Fatal, and the cause is visible as a stderr-shaped Output line.
        let fatals: Vec<String> = f
            .events()
            .into_iter()
            .filter_map(|e| match e {
                StageEvent::Fatal { message, .. } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(fatals.len(), 1, "{fatals:?}");
        assert!(
            f.events().iter().any(|e| matches!(
                e,
                StageEvent::Output {
                    stream: crate::events::Stream::Stderr,
                    ..
                }
            )),
            "the io cause must reach the user"
        );
        assert!(f.auto_fixed().is_empty(), "nothing was fixed");
        // The conf is untouched.
        assert_eq!(
            std::fs::read_to_string(&conf).unwrap(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n"
        );
    }

    // ── the die table ───────────────────────────────────────────────────────

    #[test]
    fn block_die_texts_are_run_shs_strings() {
        let f = fixture("die-table", true);
        let ctx = &f.ctx;
        let outcome = CheckOutcome::fail_bare("x", "impl message");

        for (slug, want) in [
            (
                "overlay.dxmt-d3d11",
                "CrossOver DXMT overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "overlay.dxmt-winemetal",
                "CrossOver DXMT overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "overlay.woxr-dll",
                "CrossOver wineopenxr overlay stale (CrossOver update?) — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "bottle.woxr-dll",
                "bottle wineopenxr.dll stale/missing — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "bottle.manifest",
                "bottle OpenXR manifest missing — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "bottle.registry",
                "bottle ActiveRuntime registry key missing — ./demo.sh install --bottle FixtureBottle",
            ),
            (
                "host.manifest",
                "host OpenXR registration missing — ./demo.sh install --bottle FixtureBottle",
            ),
            ("dep.goldberg", "Goldberg dll missing — ./demo.sh setup"),
        ] {
            assert_eq!(block_die(ctx, slug, &outcome).0, want, "{slug}");
        }

        // The three run-only slugs carry run.sh's die whole, in `message`.
        for slug in ["run.wine-exec", "run.bridge-built", "run.wired-adb"] {
            assert_eq!(block_die(ctx, slug, &outcome).0, "impl message", "{slug}");
        }
    }

    #[test]
    fn post_fix_die_texts_are_run_shs_strings() {
        let f = fixture("post-fix", true);
        assert_eq!(
            post_fix_die(&f.ctx, "bottle.gfx-dxmt").0,
            format!(
                "could not force graphics backend to dxmt in {}",
                f.ctx.bottle.as_ref().unwrap().conf_path().display()
            )
        );
        assert_eq!(
            post_fix_die(&f.ctx, "build.helper-staged").0,
            format!(
                "encoder helper restage failed validation at {} — ./demo.sh build",
                f.ctx.paths.oxr_helper_staged.display()
            )
        );
    }
}
