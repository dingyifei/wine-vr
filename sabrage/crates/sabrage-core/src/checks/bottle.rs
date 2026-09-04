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
mod tests;
