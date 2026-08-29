//! `fix.restage-helper` — copy the native-arm64 encoder helper from
//! `build-helper-arm64` next to the runtime dylib in `build-x64`.
//!
//! The x86_64 runtime locates the helper beside its own dylib
//! (`dladdr`/ModuleDirectory), so a `build-x64` re-configure that sweeps the
//! staged copy silently downgrades a session to in-process H.264. `run.sh`
//! self-heals by restaging; this is the same operation, offered as a fix.
//!
//! Implementation notes for the fixes agent:
//! * source is [`crate::paths::Paths::oxr_helper_built`], destination is
//!   [`crate::paths::Paths::oxr_helper_staged`];
//! * copy with the executor's `copy_if_changed` (cmp-bytes semantics — an
//!   already-current helper reports `unchanged`, and the fix reports
//!   [`FixReport::unchanged`]);
//! * verify the SOURCE is arm64 first ([`crate::util::helper_is_arm64`], where
//!   `arm64e` alone must NOT satisfy) and refuse rather than stage a wrong-arch
//!   binary that would shadow a good one.
//!
//! Ported verbatim from `run.sh`'s `ensure_helper_staged()` (lines 72–91):
//!
//! ```zsh
//! ensure_helper_staged() {
//!   helper_is_arm64 "$OXR_HELPER_BIN" && return 0
//!   if helper_is_arm64 "$OXR_HELPER_BIN_BUILT"; then
//!     warn "encoder helper missing/not arm64 at $OXR_HELPER_BIN — restaging from the helper build tree"
//!     install_if_changed "$OXR_HELPER_BIN_BUILT" "$OXR_HELPER_BIN"
//!     helper_is_arm64 "$OXR_HELPER_BIN" || \
//!       die "encoder helper restage failed validation at $OXR_HELPER_BIN — ./demo.sh build"
//!     ok "encoder helper restaged (arm64)"
//!   else
//!     die "encoder_process=$ENCODER_PROC needs the arm64 helper, but neither the staged copy
//!        ($OXR_HELPER_BIN) nor the build output ($OXR_HELPER_BIN_BUILT) is an arm64 executable — ./demo.sh build"
//!   fi
//! }
//! ```
//!
//! `$ENCODER_PROC` (default `"auto"` when the key is absent/empty, exactly like
//! the shell's `${ENCODER_PROC:-auto}`) is read only to interpolate the final
//! die text — it does not gate whether a restage is attempted, unlike `run.sh`'s
//! outer `case`, which skips `ensure_helper_staged` entirely for
//! `encoder_process=inproc`. That skip is a *launch preflight* shortcut (inproc
//! never needs the helper); this fix is invoked independently of any launch —
//! from `build`'s own arch gate, or a GUI button — so it always attempts the
//! restage the shell function's body describes.

use crate::error::Result;
use crate::events::{step, StageEvent, StepId};
use crate::executor::Copied;
use crate::fixes::{FixAction, FixReport};
use crate::stages::{EventSink, StageCtx};
use crate::util::helper_is_arm64;

/// `fix.restage-helper`'s own step id for [`crate::events::StageEvent::Line`]
/// rows that aren't naturally part of a `build` stage run (the arm64
/// re-validation failure, the final "needs the arm64 helper" die). The copy
/// itself is attributed to [`step::BUILD_HELPER`] — restaging the encoder
/// helper is exactly what that step means, whether triggered by `build` or by
/// this fix.
const STEP: StepId = "fix.restage-helper";

/// Same algorithm as `checks::config`'s (private) `parse_protocol`, for the
/// `encoder_process` key instead of `protocol`:
/// `awk -F'"' '/^[[:space:]]*encoder_process[[:space:]]*=/{print $2; exit}' "$TOML"`.
/// Duplicated rather than shared because that parser is private to a module
/// this fix does not own (file-ownership split, design-core §9).
fn parse_encoder_process(toml_text: &str) -> String {
    for line in toml_text.lines() {
        let after_leading_ws = line.trim_start();
        let Some(rest) = after_leading_ws.strip_prefix("encoder_process") else {
            continue;
        };
        if !rest.trim_start().starts_with('=') {
            continue;
        }
        let mut fields = line.split('"');
        let _before_first_quote = fields.next();
        return fields.next().unwrap_or("").to_string();
    }
    String::new()
}

/// `${ENCODER_PROC:-auto}` — empty (missing key, missing file, unquoted value)
/// falls back to `"auto"`, exactly like the shell parameter expansion.
fn encoder_process_or_default(toml_text: &str) -> String {
    let raw = parse_encoder_process(toml_text);
    if raw.is_empty() {
        "auto".to_string()
    } else {
        raw
    }
}

/// Stage the arm64 helper next to the runtime dylib.
pub async fn restage_helper(ctx: &StageCtx, sink: &EventSink) -> Result<FixReport> {
    let staged = ctx.paths.oxr_helper_staged.clone();
    let built = ctx.paths.oxr_helper_built.clone();

    // `helper_is_arm64 "$OXR_HELPER_BIN" && return 0` — completely silent,
    // nothing to do.
    if helper_is_arm64(&staged) {
        return Ok(FixReport::unchanged(
            FixAction::RestageHelper,
            format!("{} is already an arm64 executable", staged.display()),
        ));
    }

    if !helper_is_arm64(&built) {
        let encoder_process = {
            let text = std::fs::read_to_string(&ctx.paths.toml_path).unwrap_or_default();
            encoder_process_or_default(&text)
        };
        return Err(ctx.fatal(
            format!(
                "encoder_process={encoder_process} needs the arm64 helper, but neither the \
                 staged copy\n       ({}) nor the build output ({}) is an arm64 executable — \
                 ./demo.sh build",
                staged.display(),
                built.display()
            ),
            None,
        ));
    }

    sink(StageEvent::warn(
        ctx.run_id,
        Some(STEP),
        format!(
            "encoder helper missing/not arm64 at {} — restaging from the helper build tree",
            staged.display()
        ),
    ));

    let dry_run = ctx.executor.is_dry_run();
    let executor = ctx.executor_for(step::BUILD_HELPER);
    let copied = executor.copy_if_changed(&built, &staged).await?;
    match copied {
        // `install_if_changed`'s "unchanged" branch is trustworthy even in a
        // dry run (the executor's dry-run `copy_if_changed` still does the
        // real byte compare) — this can only actually happen here if the
        // staged copy became arm64-but-not-executable between the two probes
        // above and this one, but the row is reproduced regardless, matching
        // `install_if_changed` unconditionally.
        Copied::Unchanged => sink(StageEvent::info(
            ctx.run_id,
            Some(step::BUILD_HELPER),
            format!("unchanged: {}", staged.display()),
        )),
        Copied::Copied => {
            let verb = if dry_run {
                "would install"
            } else {
                "installed"
            };
            sink(StageEvent::ok(
                ctx.run_id,
                Some(step::BUILD_HELPER),
                format!("{verb}: {}", staged.display()),
            ));
        }
    }

    // A dry run never actually wrote the file, so re-validating it here would
    // always (wrongly) look like a failed restage. Skip the check and the
    // "restaged" claim accordingly; the executor's plan already records what
    // would have happened.
    if dry_run {
        let description = "encoder helper would be restaged (arm64)".to_string();
        sink(StageEvent::ok(ctx.run_id, Some(STEP), description.clone()));
        return Ok(FixReport::changed(FixAction::RestageHelper, description));
    }

    if !helper_is_arm64(&staged) {
        return Err(ctx.fatal(
            format!(
                "encoder helper restage failed validation at {} — ./demo.sh build",
                staged.display()
            ),
            None,
        ));
    }

    let description = "encoder helper restaged (arm64)".to_string();
    sink(StageEvent::ok(ctx.run_id, Some(STEP), description.clone()));
    Ok(FixReport::changed(FixAction::RestageHelper, description))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use crate::stages::{StageCtx, StageOptions};
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio_util::sync::CancellationToken;

    // ── parse_encoder_process / encoder_process_or_default ──────────────────

    #[test]
    fn parse_encoder_process_matches_the_awk_recipe() {
        assert_eq!(
            parse_encoder_process("encoder_process = \"native\"\n"),
            "native"
        );
        assert_eq!(
            parse_encoder_process("  encoder_process=\"auto\"\n"),
            "auto"
        );
        assert_eq!(
            parse_encoder_process("# encoder_process = \"native\"\n"),
            ""
        );
        assert_eq!(parse_encoder_process("encoder_process_extra = \"x\"\n"), "");
        assert_eq!(parse_encoder_process("encoder_process=native\n"), "");
        assert_eq!(
            parse_encoder_process("protocol = \"alvr\"\nencoder_process = \"inproc\"\n"),
            "inproc"
        );
        assert_eq!(parse_encoder_process(""), "");
    }

    #[test]
    fn encoder_process_or_default_falls_back_to_auto() {
        assert_eq!(encoder_process_or_default(""), "auto");
        assert_eq!(encoder_process_or_default("protocol = \"alvr\"\n"), "auto");
        assert_eq!(
            encoder_process_or_default("encoder_process = \"\"\n"),
            "auto"
        );
        assert_eq!(
            encoder_process_or_default("encoder_process = \"native\"\n"),
            "native"
        );
    }

    // ── restage_helper ────────────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("sabrage-helper-fix-{tag}-{}", std::process::id()))
    }

    fn write_thin_arm64_stub(path: &Path) {
        // Use the CURRENT test binary's own bytes as a real, verifiable
        // thin-arm64 (or whatever this machine's build actually is) Mach-O —
        // exactly the trick `checks::build`'s own tests use. Skipped by
        // callers on a non-arm64 build machine (see `arm64_available` below).
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), path).unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    /// Whether this machine's test binary is itself arm64 — the same
    /// precondition `checks::build`'s `helper_is_arm64` tests gate on. Skip
    /// (rather than fail) elsewhere, matching that module's convention.
    fn arm64_available() -> bool {
        helper_is_arm64(&std::env::current_exe().unwrap())
    }

    fn ctx_for(root: &Path, dry_run: bool) -> (StageCtx, Arc<StdMutex<Vec<StageEvent>>>) {
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let s = seen.clone();
        let sink: EventSink = Arc::new(move |ev| s.lock().unwrap().push(ev));
        let opts = StageOptions {
            dry_run,
            ..StageOptions::default()
        };
        let ctx = StageCtx::new(Paths::new(root), opts, sink, CancellationToken::new());
        (ctx, seen)
    }

    #[tokio::test]
    async fn already_staged_arm64_is_a_silent_noop() {
        if !arm64_available() {
            return;
        }
        let root = scratch("noop");
        let (ctx, seen) = ctx_for(&root, false);
        write_thin_arm64_stub(&ctx.paths.oxr_helper_staged);

        let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
        assert!(!report.changed);
        assert!(
            seen.lock().unwrap().is_empty(),
            "run.sh prints nothing here"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn restages_from_the_build_tree_when_staged_is_missing() {
        if !arm64_available() {
            return;
        }
        let root = scratch("restage");
        let (ctx, seen) = ctx_for(&root, false);
        write_thin_arm64_stub(&ctx.paths.oxr_helper_built);
        // The staged FILE is intentionally absent, but its directory (as it
        // would be after `./demo.sh build` populated `build-x64/runtime/`)
        // already exists — `copy_if_changed` does not `mkdir -p` for you, any
        // more than `cp` does.
        std::fs::create_dir_all(ctx.paths.oxr_helper_staged.parent().unwrap()).unwrap();

        let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
        assert!(report.changed);
        assert_eq!(report.description, "encoder helper restaged (arm64)");
        assert!(ctx.paths.oxr_helper_staged.is_file());
        assert!(helper_is_arm64(&ctx.paths.oxr_helper_staged));

        let evs = seen.lock().unwrap();
        let texts: Vec<&str> = evs
            .iter()
            .filter_map(|e| match e {
                StageEvent::Line { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts[0].starts_with("encoder helper missing/not arm64 at"));
        assert!(texts[0].ends_with("restaging from the helper build tree"));
        assert!(texts.iter().any(|t| t.starts_with("installed: ")));
        assert_eq!(*texts.last().unwrap(), "encoder helper restaged (arm64)");

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dry_run_reports_would_restage_without_writing_or_re_validating() {
        if !arm64_available() {
            return;
        }
        let root = scratch("dry");
        let (ctx, _seen) = ctx_for(&root, true);
        write_thin_arm64_stub(&ctx.paths.oxr_helper_built);

        let report = restage_helper(&ctx, &ctx.sink.clone()).await.unwrap();
        assert!(report.changed);
        assert_eq!(
            report.description,
            "encoder helper would be restaged (arm64)"
        );
        assert!(
            !ctx.paths.oxr_helper_staged.exists(),
            "dry run must never write"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn neither_staged_nor_built_is_arm64_dies_with_the_verbatim_message() {
        let root = scratch("fatal");
        let toml_dir = root.join("OXRSys");
        std::fs::create_dir_all(&toml_dir).unwrap();
        let mut paths = Paths::new(&root);
        paths.toml_path = toml_dir.join("oxrsys-runtime.toml");
        std::fs::write(&paths.toml_path, "encoder_process = \"native\"\n").unwrap();
        // Neither oxr_helper_staged nor oxr_helper_built exists at all.

        let opts = StageOptions::default();
        let sink: EventSink = Arc::new(|_| {});
        let ctx = StageCtx::new(paths, opts, sink.clone(), CancellationToken::new());
        let err = restage_helper(&ctx, &sink).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.starts_with(
            "encoder_process=native needs the arm64 helper, but neither the staged copy"
        ));
        assert!(msg.contains(&format!(
            "\n       ({}) nor the build output ({}) is an arm64 executable — ./demo.sh build",
            ctx.paths.oxr_helper_staged.display(),
            ctx.paths.oxr_helper_built.display()
        )));

        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn missing_toml_defaults_encoder_process_to_auto_in_the_die_text() {
        let root = scratch("fatal-no-toml");
        let opts = StageOptions::default();
        let sink: EventSink = Arc::new(|_| {});
        let ctx = StageCtx::new(
            Paths::new(&root),
            opts,
            sink.clone(),
            CancellationToken::new(),
        );
        let err = restage_helper(&ctx, &sink).await.unwrap_err();
        assert!(err
            .to_string()
            .starts_with("encoder_process=auto needs the arm64 helper"));
        std::fs::remove_dir_all(&root).ok();
    }
}
