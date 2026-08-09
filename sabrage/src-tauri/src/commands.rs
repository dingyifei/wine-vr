//! Tauri commands for the Doctor screen and the sidebar's app-state footer.
//!
//! Bridges `sabrage-core`'s synchronous, read-only check engine
//! ([`sabrage_core::checks::run_doctor`]) to the frontend: `run_doctor` streams
//! one [`DoctorEvent`] per resolved `CheckOutcome` over an IPC [`Channel`] and
//! resolves to the aggregate [`DoctorSummary`]; `get_app_state` is the small
//! always-fresh snapshot the sidebar footer renders.
//!
//! `ui/src/ipc.ts` hand-mirrors every serde shape here 1:1 — keep both sides in
//! sync when either changes.

use std::env;
use std::path::{Path, PathBuf};

use sabrage_core::checks::{run_doctor as core_run_doctor, CheckCtx, CheckOptions, CheckStatus};
use sabrage_core::{contract, Paths};
use serde::Serialize;
use tauri::ipc::Channel;

/// The ALVR client version this wine-vr checkout is pinned to (`CLAUDE.md`'s
/// submodule table). Not read from anywhere at runtime — like
/// `contract/pipeline.toml`'s baked-in pins, it is a fact about this checkout,
/// not machine state.
const ALVR_VERSION: &str = "v20.14.1";

/// One streamed doctor row: a `CheckOutcome` plus the `group` the contract
/// attaches to its `slug` (`CheckOutcome` itself carries no group — see
/// `checks/mod.rs`'s doc comment on the group → module mapping).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorEvent {
    pub slug: String,
    pub group: String,
    pub status: CheckStatus,
    pub message: String,
    pub remedy: Option<String>,
    pub detail: Option<String>,
}

/// The aggregate a `run_doctor` invocation resolves to, over the same
/// doctor-row set streamed on the channel (i.e. `run-only` excluded).
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSummary {
    pub fail_count: usize,
    pub warn_count: usize,
    pub total: usize,
}

/// Sidebar footer snapshot.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub repo_root: Option<String>,
    pub bottles: Vec<String>,
    pub alvr_version: String,
}

/// Run every doctor check in contract order, streaming each resolved row to
/// `on_event` as it settles, and return the aggregate.
///
/// Runs on a blocking task: every evaluator is a synchronous, read-only probe
/// (a stat, a small read, a digest, or a short subprocess —
/// [`sabrage_core::checks::Evaluator`]'s own doc comment), several of which
/// shell out (`adb devices`, `SwitchAudioSource`), so this must not run
/// directly on an async-runtime worker.
#[tauri::command]
pub async fn run_doctor(
    bottle: Option<String>,
    bs_dir: Option<String>,
    on_event: Channel<DoctorEvent>,
) -> Result<DoctorSummary, String> {
    let repo_root = match resolve_repo_root() {
        Ok(p) => p,
        Err(message) => {
            // Surface the failure on the channel too, so a listener that only
            // watches the stream (not the invoke() rejection) still learns why
            // zero rows arrived.
            let _ = on_event.send(DoctorEvent {
                slug: "meta.repo-root".to_string(),
                group: "meta".to_string(),
                status: CheckStatus::Fail,
                message: message.clone(),
                remedy: Some(
                    "set SABRAGE_REPO_ROOT to the wine-vr checkout, or run Sabrage from a \
                     build under that checkout"
                        .to_string(),
                ),
                detail: None,
            });
            return Err(message);
        }
    };

    // WINEVR_* env is the base (parity with the CLI and demo.sh precedence);
    // explicit GUI args override.
    let mut opts = CheckOptions::from_env();
    if let Some(b) = bottle {
        opts.bottle_name = Some(b);
    }
    if let Some(d) = bs_dir {
        opts.bs_dir_override = Some(PathBuf::from(d));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let ctx = CheckCtx::new(Paths::new(&repo_root), opts);
        let mut fail_count = 0usize;
        let mut warn_count = 0usize;
        let mut total = 0usize;
        // run-only filtering lives in sabrage-core's run_doctor (the one policy
        // site) — every outcome that reaches this sink is a doctor row.
        core_run_doctor(&ctx, |outcome| {
            let group = contract()
                .check(&outcome.slug)
                .map(|spec| spec.group.as_str())
                .unwrap_or("")
                .to_string();
            total += 1;
            if outcome.status.counts_as_fail() {
                fail_count += 1;
            }
            if outcome.status.counts_as_warn() {
                warn_count += 1;
            }
            let _ = on_event.send(DoctorEvent {
                slug: outcome.slug,
                group,
                status: outcome.status,
                message: outcome.message,
                remedy: outcome.remedy,
                detail: outcome.detail,
            });
        });
        DoctorSummary {
            fail_count,
            warn_count,
            total,
        }
    })
    .await
    .map_err(|e| format!("doctor check task did not complete: {e}"))
}

/// Sidebar footer snapshot: repo root (if resolvable), bottles present on this
/// machine, and the pinned ALVR client version.
#[tauri::command]
pub fn get_app_state() -> AppState {
    AppState {
        repo_root: resolve_repo_root().ok().map(|p| p.display().to_string()),
        bottles: sabrage_core::paths::list_bottles(),
        alvr_version: ALVR_VERSION.to_string(),
    }
}

// ── repo root resolution ─────────────────────────────────────────────────────
//
// `SABRAGE_REPO_ROOT` env var, else walk up from the running executable
// looking for the `demo.sh` + `scripts/demo/lib.sh` pair that identifies the
// wine-vr checkout, else a clear error — the same shape as
// `sabrage-cli/src/main.rs`'s private `resolve_repo_root`.
//
// Duplicated rather than reused: `sabrage-cli` is a `[[bin]]`-only crate (see
// its Cargo.toml's Frame-agent comment — `sabrage-core` is its one deliberate
// dependency) with no library target for `src-tauri` to depend on, and adding
// one is a Cargo.toml edit outside this agent's authority. Flagged in the
// final report rather than worked around silently.

fn resolve_repo_root() -> Result<PathBuf, String> {
    if let Some(over) = env::var("SABRAGE_REPO_ROOT").ok().filter(|s| !s.is_empty()) {
        // Canonicalize so a symlinked/`..` override still satisfies
        // host.manifest's exact string equality on library_path.
        let p = PathBuf::from(over);
        return Ok(p.canonicalize().unwrap_or(p));
    }
    let exe = env::current_exe()
        .map_err(|e| format!("cannot resolve Sabrage's own executable path: {e}"))?;
    find_repo_root_from_exe(&exe).ok_or_else(|| {
        format!(
            "could not locate the wine-vr repo root (looked for demo.sh + \
             scripts/demo/lib.sh in every directory above {}); set SABRAGE_REPO_ROOT \
             to override",
            exe.display()
        )
    })
}

/// Walk `exe`'s ancestors (not `exe` itself — it need not exist for this to be
/// tested) for the first directory containing both `demo.sh` and
/// `scripts/demo/lib.sh`.
fn find_repo_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if d.join("demo.sh").is_file() && d.join("scripts/demo/lib.sh").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real_repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR here is `<repo>/sabrage/src-tauri` — two levels
        // below the repo root (unlike sabrage-cli, one crate deeper at
        // `<repo>/sabrage/crates/sabrage-cli`, which needs three).
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repo root resolves from CARGO_MANIFEST_DIR")
    }

    #[test]
    fn finds_repo_root_by_walking_up_from_a_plausible_exe_path() {
        let root = real_repo_root();
        let fake_exe = root.join("target/debug/sabrage-app");
        assert_eq!(find_repo_root_from_exe(&fake_exe), Some(root.clone()));

        let deeper = root.join("some/nested/install/dir/Sabrage.app/Contents/MacOS/sabrage-app");
        assert_eq!(find_repo_root_from_exe(&deeper), Some(root));
    }

    #[test]
    fn returns_none_when_nothing_above_has_the_pair() {
        assert_eq!(
            find_repo_root_from_exe(Path::new("/nonexistent/sabrage/bin/sabrage")),
            None
        );
    }

    #[test]
    fn no_doctor_row_group_matches_the_contracts_run_only_slugs() {
        use sabrage_core::checks::NO_DOCTOR_ROW_GROUP;
        let c = contract();
        assert_eq!(
            c.check("run.wine-exec").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
        assert_eq!(
            c.check("run.bridge-built").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
        // A doctor-visible slug, for contrast.
        assert_ne!(
            c.check("sys.arch").expect("slug present").group,
            NO_DOCTOR_ROW_GROUP
        );
    }
}
