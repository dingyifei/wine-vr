//! Group `bottle` — doctor.sh section 3: resolves the bottle context every
//! later section consumes. Slug order pinned by
//! checks::tests::registry_binds_in_contract_order_and_covers_every_slug.
//!
//! Every evaluator is a read-only `fn(&CheckCtx) -> CheckOutcome`.
//! `bottle.template`/`bottle.gfx-dxmt`/`bottle.zdrive` report
//! [`CheckStatus::Skipped`] whenever `ctx.bottle` is `None` — see
//! `skip_reason_for_missing_bottle` for the reason text and
//! tests::bottle_named_fail_without_a_name_skips_the_rest_of_the_section.

use std::path::Path;

use super::Evaluator;
#[allow(unused_imports)]
use super::{CheckCtx, CheckOutcome, CheckStatus, SkipReason};
use crate::paths::Bottle;

/// `create a win11_64 bottle in CrossOver; existing: $(ls … | tr '\n' ' ')`.
///
/// [`crate::paths::list_bottles`] is the typed port of that same `ls`, so the
/// only thing left to reproduce here is `tr`'s effect: each entry gets a
/// trailing space (including the last), and an empty list stays empty.
fn bottle_named_remedy() -> String {
    let names = crate::paths::list_bottles();
    let existing = if names.is_empty() {
        String::new()
    } else {
        format!("{} ", names.join(" "))
    };
    format!("create a win11_64 bottle in CrossOver; existing: {existing}")
}

/// Why `bottle.template`/`bottle.gfx-dxmt`/`bottle.zdrive` are skipped: either
/// no name was given at all, or the named bottle doesn't resolve.
fn skip_reason_for_missing_bottle(ctx: &CheckCtx) -> SkipReason {
    if ctx.bottle_requested {
        SkipReason::new(format!("bottle '{}' not found", ctx.bottle_label()))
    } else {
        SkipReason::new("no bottle name given")
    }
}

/// `[[ "$BS_DIR" != "$PREFIX/drive_c/"* ]]` — string-prefix test with a
/// trailing slash, same nuance as `win_path`: the bare `<prefix>/drive_c`
/// directory itself does not match.
fn bs_dir_outside_drive_c(bottle: &Bottle, bs_dir: &Path) -> bool {
    let glob = format!("{}/drive_c/", bottle.prefix.display());
    !bs_dir.to_string_lossy().starts_with(&glob)
}

fn bottle_named(ctx: &CheckCtx) -> CheckOutcome {
    if ctx.bottle_requested {
        // Silent when clean on the shell side (`tap bottle.named ok`) — no
        // verbatim string to match; the CLI console suppresses the row.
        CheckOutcome::silent_pass(
            "bottle.named",
            format!("bottle name given: {}", ctx.bottle_label()),
        )
    } else {
        CheckOutcome::fail(
            "bottle.named",
            "no bottle name given (--bottle/WINEVR_BOTTLE)",
            bottle_named_remedy(),
        )
    }
}

fn bottle_exists(ctx: &CheckCtx) -> CheckOutcome {
    if !ctx.bottle_requested {
        return CheckOutcome::skipped("bottle.exists", SkipReason::new("no bottle name given"));
    }
    match &ctx.bottle {
        Some(_) => CheckOutcome::pass(
            "bottle.exists",
            format!("bottle '{}' exists", ctx.bottle_label()),
        ),
        None => {
            let b = Bottle::unvalidated(ctx.bottle_label());
            CheckOutcome::fail(
                "bottle.exists",
                format!("bottle '{}' not found at {}", b.name, b.prefix.display()),
                "create it in the CrossOver UI (win11_64)",
            )
        }
    }
}

fn bottle_template(ctx: &CheckCtx) -> CheckOutcome {
    let Some(b) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.template", skip_reason_for_missing_bottle(ctx));
    };
    let conf = std::fs::read_to_string(b.conf_path()).unwrap_or_default();
    // `grep -q '^"Template" = "win11_64"'` — prefix match, no `$` anchor: a
    // line may have trailing content and still count.
    if conf
        .lines()
        .any(|l| l.starts_with("\"Template\" = \"win11_64\""))
    {
        CheckOutcome::pass("bottle.template", "bottle template win11_64")
    } else {
        // `grep '^"Template"' … | head -1` — first line starting with the
        // bare key, verbatim (empty when the key is absent entirely).
        let found = conf
            .lines()
            .find(|l| l.starts_with("\"Template\""))
            .unwrap_or("");
        CheckOutcome::warn(
            "bottle.template",
            format!("bottle template is not win11_64 ({found}) — only win11_64 is verified"),
        )
    }
}

fn bottle_gfx_dxmt(ctx: &CheckCtx) -> CheckOutcome {
    let Some(b) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.gfx-dxmt", skip_reason_for_missing_bottle(ctx));
    };
    let conf = std::fs::read_to_string(b.conf_path()).unwrap_or_default();
    // `grep -q '^"CX_GRAPHICS_BACKEND" = "dxmt"$'` — anchored at both ends:
    // an exact full-line match, unlike bottle.template's prefix match. The
    // same literal `fixes::backend::TARGET_LINE` writes.
    if conf
        .lines()
        .any(|l| l == crate::fixes::backend::TARGET_LINE)
    {
        CheckOutcome::pass("bottle.gfx-dxmt", "bottle graphics backend = dxmt")
    } else {
        CheckOutcome::fail(
            "bottle.gfx-dxmt",
            "bottle graphics backend is not dxmt (the CrossOver GUI 'auto' setting no longer \
             selects DXMT — game stalls before D3D11 init, streamer never starts)",
            "./demo.sh run auto-fixes this, or set Graphics Backend to DXMT in the CrossOver \
             bottle settings",
        )
    }
}

fn bottle_zdrive(ctx: &CheckCtx) -> CheckOutcome {
    let Some(b) = &ctx.bottle else {
        return CheckOutcome::skipped("bottle.zdrive", skip_reason_for_missing_bottle(ctx));
    };
    if !bs_dir_outside_drive_c(b, &ctx.bs_dir) {
        return CheckOutcome::skipped(
            "bottle.zdrive",
            SkipReason::new("Beat Saber install is inside drive_c"),
        );
    }
    if b.z_drive().exists() {
        CheckOutcome::pass(
            "bottle.zdrive",
            "bottle z: drive maps / (Beat Saber lives outside drive_c)",
        )
    } else {
        CheckOutcome::fail(
            "bottle.zdrive",
            "Beat Saber is outside drive_c but the bottle has no z: drive",
            "add dosdevices/z: -> / or move the install under drive_c",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("bottle.named", bottle_named as Evaluator),
        ("bottle.exists", bottle_exists as Evaluator),
        ("bottle.template", bottle_template as Evaluator),
        ("bottle.gfx-dxmt", bottle_gfx_dxmt as Evaluator),
        ("bottle.zdrive", bottle_zdrive as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::CheckOptions;
    use crate::paths::Paths;
    use std::fs;
    use std::path::PathBuf;

    fn ctx_with(bottle: Option<Bottle>, bottle_requested: bool, bs_dir: PathBuf) -> CheckCtx {
        CheckCtx {
            paths: Paths::new("/nonexistent/repo"),
            bottle,
            bs_dir,
            bottle_requested,
            opts: CheckOptions {
                bottle_name: if bottle_requested {
                    Some("TestBottle".into())
                } else {
                    None
                },
                ..CheckOptions::new()
            },
        }
    }

    #[test]
    fn bottle_named_fail_without_a_name_skips_the_rest_of_the_section() {
        let ctx = ctx_with(None, false, PathBuf::from("/bs"));
        let out = bottle_named(&ctx);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(out.message, "no bottle name given (--bottle/WINEVR_BOTTLE)");
        assert!(out
            .remedy
            .as_deref()
            .unwrap()
            .starts_with("create a win11_64 bottle in CrossOver; existing: "));

        assert_eq!(bottle_exists(&ctx).status, CheckStatus::Skipped);
        assert_eq!(bottle_template(&ctx).status, CheckStatus::Skipped);
        assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Skipped);
        assert_eq!(bottle_zdrive(&ctx).status, CheckStatus::Skipped);
    }

    #[test]
    fn bottle_exists_fail_reports_prefix_and_skips_downstream_rows() {
        let ctx = ctx_with(None, true, PathBuf::from("/bs"));
        let out = bottle_exists(&ctx);
        let want_prefix = Bottle::unvalidated("TestBottle").prefix;
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.message,
            format!("bottle 'TestBottle' not found at {}", want_prefix.display())
        );
        assert_eq!(
            out.remedy.as_deref(),
            Some("create it in the CrossOver UI (win11_64)")
        );
        assert_eq!(bottle_template(&ctx).status, CheckStatus::Skipped);
        assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Skipped);
        assert_eq!(bottle_zdrive(&ctx).status, CheckStatus::Skipped);
    }

    fn temp_bottle(label: &str) -> Bottle {
        let dir = std::env::temp_dir().join(format!(
            "sabrage-bottle-test-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp bottle dir");
        Bottle {
            name: "TestBottle".to_string(),
            sys32: dir.join("drive_c/windows/system32"),
            prefix: dir,
        }
    }

    #[test]
    fn bottle_template_and_gfx_dxmt_pass_on_a_matching_conf() {
        let b = temp_bottle("conf-ok");
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win11_64\"\n\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n",
        )
        .unwrap();
        let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
        assert_eq!(bottle_template(&ctx).status, CheckStatus::Pass);
        assert_eq!(bottle_gfx_dxmt(&ctx).status, CheckStatus::Pass);
        fs::remove_dir_all(&b.prefix).ok();
    }

    #[test]
    fn bottle_template_warns_and_gfx_dxmt_fails_on_mismatch() {
        let b = temp_bottle("conf-bad");
        fs::write(
            b.conf_path(),
            "\"Template\" = \"win10_64\"\n\"CX_GRAPHICS_BACKEND\" = \"auto\"\n",
        )
        .unwrap();
        let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));

        let t = bottle_template(&ctx);
        assert_eq!(t.status, CheckStatus::Warn);
        assert_eq!(
            t.message,
            "bottle template is not win11_64 (\"Template\" = \"win10_64\") — only win11_64 is verified"
        );
        assert!(t.remedy.is_none());

        let g = bottle_gfx_dxmt(&ctx);
        assert_eq!(g.status, CheckStatus::Fail);
        assert_eq!(
            g.remedy.as_deref(),
            Some(
                "./demo.sh run auto-fixes this, or set Graphics Backend to DXMT in the \
                 CrossOver bottle settings"
            )
        );
        fs::remove_dir_all(&b.prefix).ok();
    }

    #[test]
    fn bottle_template_warns_with_an_empty_parenthetical_when_the_key_is_absent() {
        let b = temp_bottle("conf-no-template");
        fs::write(b.conf_path(), "\"CX_GRAPHICS_BACKEND\" = \"dxmt\"\n").unwrap();
        let ctx = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
        let t = bottle_template(&ctx);
        assert_eq!(
            t.message,
            "bottle template is not win11_64 () — only win11_64 is verified"
        );
        fs::remove_dir_all(&b.prefix).ok();
    }

    #[test]
    fn bottle_zdrive_gates_on_outside_drive_c_and_z_drive_presence() {
        let b = temp_bottle("zdrive");
        fs::create_dir_all(b.prefix.join("drive_c")).unwrap();

        // Inside drive_c: skipped regardless of z: presence.
        let ctx_inside = ctx_with(Some(b.clone()), true, b.prefix.join("drive_c/bs"));
        assert_eq!(bottle_zdrive(&ctx_inside).status, CheckStatus::Skipped);

        // Outside drive_c, no z: -> fail.
        let ctx_outside = ctx_with(Some(b.clone()), true, PathBuf::from("/elsewhere/bs"));
        let out = bottle_zdrive(&ctx_outside);
        assert_eq!(out.status, CheckStatus::Fail);
        assert_eq!(
            out.remedy.as_deref(),
            Some("add dosdevices/z: -> / or move the install under drive_c")
        );

        // Outside drive_c, z: present -> pass.
        fs::create_dir_all(b.prefix.join("dosdevices")).unwrap();
        fs::write(b.prefix.join("dosdevices/z:"), b"").unwrap();
        let out = bottle_zdrive(&ctx_outside);
        assert_eq!(
            out.status,
            CheckStatus::Pass,
            "z: drive now present, outside drive_c"
        );
        assert_eq!(
            out.message,
            "bottle z: drive maps / (Beat Saber lives outside drive_c)"
        );

        fs::remove_dir_all(&b.prefix).ok();
    }
}
