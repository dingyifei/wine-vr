//! `win_path()` — the unix→Windows path rule the run stage hands to wine.
//!
//! lib.sh:
//!
//! ```zsh
//! win_path() {
//!   if [ -n "${PREFIX:-}" ] && [[ "$1" == "$PREFIX/drive_c/"* ]]; then
//!     local rel="${1#$PREFIX/drive_c/}"
//!     print -r -- "C:\\${rel//\//\\}"
//!   else
//!     print -r -- "Z:${1//\//\\}"
//!   fi
//! }
//! ```
//!
//! Two semantics are load-bearing and must not be "cleaned up"
//! (design-core §10 parity decision 22); both are pinned by
//! `sabrage-parity`'s `win_path_table`:
//!
//! 1. The glob is `"$PREFIX/drive_c/"*` — a **trailing slash**. So the literal
//!    directory `<prefix>/drive_c` (no trailing slash, nothing after it) does
//!    *not* match and falls through to the `Z:` branch. This is string matching,
//!    not path-component matching: `PathBuf::starts_with` would wrongly accept it.
//! 2. The `Z:` branch prefixes the **whole** unix path (leading `/` included) and
//!    only translates separators, producing e.g. `Z:\Users\me\game`.

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
