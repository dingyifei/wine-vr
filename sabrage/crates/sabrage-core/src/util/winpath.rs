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
//! (design-core §10 parity decision 22):
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn prefix() -> PathBuf {
        PathBuf::from("/Users/me/Library/Application Support/CrossOver/Bottles/Steam")
    }

    #[test]
    fn table() {
        let pre = prefix();
        let pre_s = pre.display().to_string();

        // Inside drive_c -> C:\ with separators flipped.
        assert_eq!(
            win_path(
                Some(&pre),
                Path::new(&format!("{pre_s}/drive_c/windows/system32/wineopenxr.dll"))
            ),
            "C:\\windows\\system32\\wineopenxr.dll"
        );
        // Spaces survive untouched.
        assert_eq!(
            win_path(
                Some(&pre),
                Path::new(&format!(
                    "{pre_s}/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"
                ))
            ),
            "C:\\Program Files (x86)\\Steam\\steamapps\\common\\Beat Saber 1294"
        );
        // Immediate child of drive_c.
        assert_eq!(
            win_path(Some(&pre), Path::new(&format!("{pre_s}/drive_c/openxr"))),
            "C:\\openxr"
        );
        // PARITY: the literal drive_c directory does NOT match the trailing-slash
        // glob and falls through to Z:.
        assert_eq!(
            win_path(Some(&pre), Path::new(&format!("{pre_s}/drive_c"))),
            format!("Z:{}", format!("{pre_s}/drive_c").replace('/', "\\"))
        );
        // Outside the bottle -> Z: + the whole path.
        assert_eq!(
            win_path(Some(&pre), Path::new("/games/Beat Saber 1294")),
            "Z:\\games\\Beat Saber 1294"
        );
        // No prefix at all -> Z:.
        assert_eq!(win_path(None, Path::new("/games/bs")), "Z:\\games\\bs");
        // Empty prefix behaves like no prefix (zsh `[ -n "${PREFIX:-}" ]`).
        assert_eq!(
            win_path(Some(Path::new("")), Path::new("/games/bs")),
            "Z:\\games\\bs"
        );
        // A sibling directory whose name merely starts with "drive_c" is not inside it.
        assert_eq!(
            win_path(Some(&pre), Path::new(&format!("{pre_s}/drive_cache/x"))),
            format!("Z:{}", format!("{pre_s}/drive_cache/x").replace('/', "\\"))
        );
    }
}
