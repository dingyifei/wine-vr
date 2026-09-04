//! `win_path()` — the unix→Windows path rule the run stage hands to wine
//! (reference: `scripts/demo/lib.sh`).
//!
//! Load-bearing (design-core §10 parity decision 22), pinned by
//! `sabrage-parity::tests::artifact_goldens::win_path_table`: `C:` needs the trailing slash of
//! `<prefix>/drive_c/` (bare `drive_c` falls through); `Z:` prefixes the whole unix path and only
//! translates separators.

use std::path::Path;

/// Convert a unix absolute path to the Windows path wine sees.
///
/// `prefix` is the bottle prefix (`PREFIX`); `None` reproduces lib.sh's
/// `[ -n "${PREFIX:-}" ]` guard failing, i.e. everything becomes a `Z:` path.
pub fn win_path(prefix: Option<&Path>, p: &Path) -> String {
    let p_str = p.to_string_lossy();
    if let Some(prefix) = prefix {
        let prefix_str = prefix.to_string_lossy();
        // zsh: `[ -n "${PREFIX:-}" ]` — an empty PREFIX takes the Z: branch.
        if !prefix_str.is_empty() {
            let drive_c = format!("{prefix_str}/drive_c/");
            if let Some(rel) = p_str.strip_prefix(&drive_c) {
                return format!("C:\\{}", rel.replace('/', "\\"));
            }
        }
    }
    format!("Z:{}", p_str.replace('/', "\\"))
}
