//! Group `sources` — doctor.sh section 6: submodule checkouts and the ALVR patch set.
//!
//! Slugs owned here, in contract order:
//!
//! * `src.oxrsys` — `ext/oxrsys/.git` exists (file or dir)
//! * `src.wineopenxr` — `ext/wineopenxr/.git` exists
//! * `src.alvr` — `ext/ALVR/.git` exists
//! * `src.alvr-patchset` — `ext/ALVR/alvr/server_core/src/connection.rs`
//!   contains `is_streaming_nonblocking` — the pin sanity-check for the
//!   oxrsys-v20.14.1 branch
//!
//! Every evaluator is `fn(&CheckCtx) -> CheckOutcome`: a **read-only probe**.
//! Message and remedy strings must match `scripts/demo/doctor.sh` verbatim.

use std::path::Path;

use super::Evaluator;
use super::{CheckCtx, CheckOutcome};

/// doctor.sh's `[ -f "$_sm/.git" ] || [ -d "$_sm/.git" ]`: a submodule
/// checkout is "present" when it has a `.git` entry at all — a file (a real
/// git submodule) or a directory (a plain nested clone).
fn git_marker_present(submodule_dir: &Path) -> bool {
    let marker = submodule_dir.join(".git");
    marker.is_file() || marker.is_dir()
}

/// `basename $_sm` — the last path component, used in the "submodule <name>
/// present / not initialized" message.
fn basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Shared shape of the three `src.*` submodule-presence checks.
fn submodule_check(slug: &'static str, dir: &Path) -> CheckOutcome {
    let name = basename(dir);
    if git_marker_present(dir) {
        CheckOutcome::pass(slug, format!("submodule {name} present"))
    } else {
        CheckOutcome::fail(
            slug,
            format!("submodule {name} not initialized"),
            "./demo.sh setup",
        )
    }
}

fn src_oxrsys(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.oxrsys", &ctx.paths.oxrsys)
}

fn src_wineopenxr(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.wineopenxr", &ctx.paths.woxr)
}

fn src_alvr(ctx: &CheckCtx) -> CheckOutcome {
    submodule_check("src.alvr", &ctx.paths.alvr)
}

/// `grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs"`
/// — a plain substring search (the needle has no regex metacharacters, so
/// basic-grep and `str::contains` agree).
fn src_alvr_patchset(ctx: &CheckCtx) -> CheckOutcome {
    let path = ctx.paths.alvr.join("alvr/server_core/src/connection.rs");
    let present = std::fs::read(&path)
        .map(|bytes| String::from_utf8_lossy(&bytes).contains("is_streaming_nonblocking"))
        .unwrap_or(false);
    if present {
        CheckOutcome::pass("src.alvr-patchset", "ALVR oxrsys patch set present")
    } else {
        CheckOutcome::fail(
            "src.alvr-patchset",
            "ALVR submodule missing the oxrsys patches",
            "./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)",
        )
    }
}

/// Evaluators this module binds, keyed by contract slug.
pub fn defs() -> Vec<(&'static str, Evaluator)> {
    vec![
        ("src.oxrsys", src_oxrsys as Evaluator),
        ("src.wineopenxr", src_wineopenxr as Evaluator),
        ("src.alvr", src_alvr as Evaluator),
        ("src.alvr-patchset", src_alvr_patchset as Evaluator),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{CheckOptions, CheckStatus};
    use crate::paths::Paths;
    use std::fs;

    fn ctx_for(root: &Path) -> CheckCtx {
        CheckCtx::new(Paths::new(root), CheckOptions::new())
    }

    #[test]
    fn submodule_missing_fails_with_the_setup_remedy() {
        let ctx = ctx_for(Path::new("/nonexistent/sabrage-sources-probe"));
        let o = src_oxrsys(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(o.message, "submodule oxrsys not initialized");
        assert_eq!(o.remedy.as_deref(), Some("./demo.sh setup"));
    }

    #[test]
    fn submodule_present_as_a_directory_passes() {
        let tmp =
            std::env::temp_dir().join(format!("sabrage-src-test-{}-{}", std::process::id(), "dir"));
        let sub = tmp.join("ext/wineopenxr");
        fs::create_dir_all(sub.join(".git")).unwrap();
        let ctx = ctx_for(&tmp);
        let o = src_wineopenxr(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(o.message, "submodule wineopenxr present");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn submodule_present_as_a_gitlink_file_passes() {
        let tmp = std::env::temp_dir().join(format!(
            "sabrage-src-test-{}-{}",
            std::process::id(),
            "file"
        ));
        let sub = tmp.join("ext/ALVR");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(".git"), "gitdir: ../../.git/modules/ext/ALVR\n").unwrap();
        let ctx = ctx_for(&tmp);
        let o = src_alvr(&ctx);
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(o.message, "submodule ALVR present");
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn patchset_check_greps_connection_rs() {
        let tmp =
            std::env::temp_dir().join(format!("sabrage-src-test-{}-patch", std::process::id()));
        let dir = tmp.join("ext/ALVR/alvr/server_core/src");
        fs::create_dir_all(&dir).unwrap();

        // Missing file -> fail.
        let ctx = ctx_for(&tmp);
        let o = src_alvr_patchset(&ctx);
        assert_eq!(o.status, CheckStatus::Fail);
        assert_eq!(o.message, "ALVR submodule missing the oxrsys patches");
        assert_eq!(
            o.remedy.as_deref(),
            Some("./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)")
        );

        // Present but without the marker -> still fail.
        fs::write(dir.join("connection.rs"), "fn connect() {}\n").unwrap();
        let o = src_alvr_patchset(&ctx_for(&tmp));
        assert_eq!(o.status, CheckStatus::Fail);

        // Marker present -> pass.
        fs::write(
            dir.join("connection.rs"),
            "fn is_streaming_nonblocking() -> bool { true }\n",
        )
        .unwrap();
        let o = src_alvr_patchset(&ctx_for(&tmp));
        assert_eq!(o.status, CheckStatus::Pass);
        assert_eq!(o.message, "ALVR oxrsys patch set present");

        fs::remove_dir_all(&tmp).ok();
    }
}
