//! The typed port of `scripts/demo/lib.sh`'s derived-path block.
//!
//! lib.sh is declared "the single source of truth for paths, sha256 pins, and
//! helpers" (CLAUDE.md); this module is its Rust mirror. Two shell traps are
//! fixed at the type level, per design-core §1:
//!
//! * **`CX` is `Option`.** lib.sh leaves `CX_APP` unset when CrossOver is absent
//!   and then unconditionally builds `CX="${CX_APP:-}/Contents/SharedSupport/CrossOver"`,
//!   i.e. the bogus absolute path `/Contents/SharedSupport/CrossOver`. Here
//!   `cx`/`wine`/`wineserver` are `None` when CrossOver is absent — never a path
//!   that looks real and silently fails every comparison.
//! * **`adb` is `Option`.** lib.sh leaves `ADB=""` when no adb is found, and
//!   every call site has to remember `[ -n "$ADB" ]` first.
//!
//! Unlike demo.sh (`ROOT="$(dirname $0)"`), Sabrage's binary lives somewhere
//! unrelated to the repo, so `repo_root` is **explicit input** (persisted in
//! Sabrage settings). Changing it invalidates install state, because the host
//! OpenXR manifest embeds the absolute dylib path under it.

use std::path::{Path, PathBuf};

use crate::contract::contract;
use crate::error::SabrageError;

/// `$HOME`, or `/` if the environment has none (a headless edge case; every
/// derived path is then obviously wrong, which is better than panicking inside
/// a read-only probe).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `~/Library/Application Support/CrossOver/Bottles`.
pub fn bottles_root() -> PathBuf {
    home_dir().join("Library/Application Support/CrossOver/Bottles")
}

/// Bottle names present on this machine, sorted. Mirrors the
/// `ls "$HOME/Library/Application Support/CrossOver/Bottles"` in doctor's
/// `bottle.named` remedy and lib.sh's `require_bottle`.
pub fn list_bottles() -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(bottles_root()) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

/// Is `p` an existing file with any execute bit set? (`command -v` semantics for
/// an absolute path.)
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `command -v <name>`: first executable named `name` on `$PATH`.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|cand| is_executable(cand))
}

/// The typed lib.sh path set. Built once per operation; no ambient globals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `ROOT` / `WINEVR_ROOT` — explicit input, not discovered.
    pub root: PathBuf,

    /// `CX_APP` — `~/Applications/CrossOver.app` wins over `/Applications/…`.
    pub cx_app: Option<PathBuf>,
    /// `CX` — `<cx_app>/Contents/SharedSupport/CrossOver`. `None`, never bogus.
    pub cx: Option<PathBuf>,
    /// `WINE` — `<cx>/bin/wine`.
    pub wine: Option<PathBuf>,
    /// `WINESERVER` — `<cx>/bin/wineserver`.
    pub wineserver: Option<PathBuf>,

    /// `OXR_APPSUP` — `~/Library/Application Support/OXRSys`.
    pub oxr_appsup: PathBuf,
    /// `TOML` — `<oxr_appsup>/oxrsys-runtime.toml` (write-once; never regenerate).
    pub toml_path: PathBuf,
    /// `HOST_XR_JSON`, straight from the contract.
    pub host_xr_json: PathBuf,

    /// `OXRSYS` — `<root>/ext/oxrsys`.
    pub oxrsys: PathBuf,
    /// `WOXR` — `<root>/ext/wineopenxr`.
    pub woxr: PathBuf,
    /// `ALVR` — `<root>/ext/ALVR`.
    pub alvr: PathBuf,
    /// `DXMT_ART` — `<root>/ext/dxmt-artifacts`.
    pub dxmt_art: PathBuf,
    /// `GBE_DLL` — `<root>/third_party/gbe/steam_api64.dll`.
    pub gbe_dll: PathBuf,

    /// `OXR_BUILD` — `<oxrsys>/build-x64` (x86_64 + ALVR; what the pipeline installs from).
    pub oxr_build: PathBuf,
    /// `OXR_DYLIB` — the runtime dylib the host manifest points at by absolute path.
    pub oxr_dylib: PathBuf,
    /// `OXR_ALVR_DYLIB` — `libalvr_server_core.dylib`, staged beside the runtime.
    pub oxr_alvr_dylib: PathBuf,
    /// `OXR_RUNTIME_JSON` — the runtime's own manifest in the build tree.
    pub oxr_runtime_json: PathBuf,
    /// `OXR_HELPER_BUILD` — `<oxrsys>/build-helper-arm64`, the *only* tree allowed
    /// to configure `OXRSYS_BUILD_ENCODER_HELPER=ON`.
    pub oxr_helper_build: PathBuf,
    /// `OXR_HELPER_BIN_BUILT` — the helper as built in the arm64 tree.
    pub oxr_helper_built: PathBuf,
    /// `OXR_HELPER_BIN` — the helper staged next to the runtime dylib, which is
    /// where the x86_64 runtime looks for it (dladdr/ModuleDirectory).
    pub oxr_helper_staged: PathBuf,

    /// `WOXR_DLL` — the PE side of the bridge.
    pub woxr_dll: PathBuf,
    /// `WOXR_SO` — the unix side of the bridge.
    pub woxr_so: PathBuf,
    /// `ALVR_DASHBOARD_BIN` — native-arch GUI; talks to the embedded server over
    /// the contract's `dashboard_addr`, so it needs no x86_64 cross target.
    pub alvr_dashboard: PathBuf,

    /// `ADB` — SDK platform-tools wins over `$PATH`; `None`, never `""`.
    pub adb: Option<PathBuf>,
}

impl Paths {
    /// Build the path set for `repo_root`, probing the machine for CrossOver and
    /// adb exactly the way lib.sh does.
    ///
    /// Probing is read-only (three `stat`s and a `$PATH` walk).
    pub fn new(repo_root: impl Into<PathBuf>) -> Paths {
        let root: PathBuf = repo_root.into();
        let home = home_dir();

        // lib.sh: for _cx in "$HOME/Applications/CrossOver.app" "/Applications/CrossOver.app"
        let cx_app = [
            home.join("Applications/CrossOver.app"),
            PathBuf::from("/Applications/CrossOver.app"),
        ]
        .into_iter()
        .find(|p| p.is_dir());
        let cx = cx_app
            .as_ref()
            .map(|a| a.join("Contents/SharedSupport/CrossOver"));
        let wine = cx.as_ref().map(|c| c.join("bin/wine"));
        let wineserver = cx.as_ref().map(|c| c.join("bin/wineserver"));

        let oxr_appsup = home.join("Library/Application Support/OXRSys");
        let oxrsys = root.join("ext/oxrsys");
        let woxr = root.join("ext/wineopenxr");
        let alvr = root.join("ext/ALVR");
        let oxr_build = oxrsys.join("build-x64");
        let oxr_helper_build = oxrsys.join("build-helper-arm64");

        // lib.sh: SDK path first, then `command -v adb`, else empty.
        let sdk_adb = home.join("Library/Android/sdk/platform-tools/adb");
        let adb = if is_executable(&sdk_adb) {
            Some(sdk_adb)
        } else {
            which("adb")
        };

        Paths {
            toml_path: oxr_appsup.join("oxrsys-runtime.toml"),
            host_xr_json: PathBuf::from(&contract().paths.host_xr_json),
            oxr_appsup,

            dxmt_art: root.join("ext/dxmt-artifacts"),
            gbe_dll: root.join("third_party/gbe/steam_api64.dll"),

            oxr_dylib: oxr_build.join("runtime/liboxrsys-runtime.dylib"),
            oxr_alvr_dylib: oxr_build.join("runtime/libalvr_server_core.dylib"),
            oxr_runtime_json: oxr_build.join("runtime/oxrsys-runtime.json"),
            oxr_helper_built: oxr_helper_build.join("runtime/oxrsys-encoder-helper"),
            oxr_helper_staged: oxr_build.join("runtime/oxrsys-encoder-helper"),
            oxr_helper_build,
            oxr_build,

            woxr_dll: woxr.join("build/src/pe/wineopenxr.dll"),
            woxr_so: woxr.join("build/src/unix/wineopenxr.so"),
            alvr_dashboard: alvr.join("target/release/alvr_dashboard"),

            oxrsys,
            woxr,
            alvr,

            cx_app,
            cx,
            wine,
            wineserver,
            adb,
            root,
        }
    }

    /// `$CX/lib/dxmt/<rel>` — a file in CrossOver's shared DXMT overlay.
    /// `None` when CrossOver is absent (the whole overlay section is skipped).
    pub fn cx_dxmt(&self, rel: &str) -> Option<PathBuf> {
        self.cx.as_ref().map(|c| c.join("lib/dxmt").join(rel))
    }

    /// `$CX/lib/wine/<rel>` — a file in CrossOver's shared wine lib tree.
    pub fn cx_wine_lib(&self, rel: &str) -> Option<PathBuf> {
        self.cx.as_ref().map(|c| c.join("lib/wine").join(rel))
    }

    /// `$OXR_APPSUP/alvr/session.json` — ALVR's machine-local session state
    /// (the file doctor's `cfg.session-pins` inspects for stale manual IPs).
    pub fn alvr_session_json(&self) -> PathBuf {
        self.oxr_appsup.join("alvr/session.json")
    }

    /// Render a repo-relative display path the way doctor does with
    /// `"${_f#$ROOT/}"`: strip the `root/` prefix when present, else print in full.
    pub fn rel_display(&self, p: &Path) -> String {
        match p.strip_prefix(&self.root) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => p.display().to_string(),
        }
    }
}

/// A CrossOver bottle: name plus the two derived paths lib.sh's `require_bottle`
/// exports (`PREFIX`, `SYS32`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bottle {
    /// `WINEVR_BOTTLE`.
    pub name: String,
    /// `PREFIX` — `~/Library/Application Support/CrossOver/Bottles/<name>`.
    pub prefix: PathBuf,
    /// `SYS32` — `<prefix>/drive_c/windows/system32`.
    pub sys32: PathBuf,
}

impl Bottle {
    /// Build the paths for `name` **without** validating that the bottle exists.
    ///
    /// doctor needs this shape even for a missing bottle, because its
    /// `bottle.exists` FAIL message quotes `$PREFIX`.
    pub fn unvalidated(name: impl Into<String>) -> Bottle {
        let name = name.into();
        let prefix = bottles_root().join(&name);
        let sys32 = prefix.join("drive_c/windows/system32");
        Bottle {
            name,
            prefix,
            sys32,
        }
    }

    /// `<prefix>/cxbottle.conf` — the file whose presence *defines* an existing
    /// bottle for both front-ends.
    pub fn conf_path(&self) -> PathBuf {
        self.prefix.join("cxbottle.conf")
    }

    /// `<prefix>/system.reg` — where the `ActiveRuntime` key lands (wine flushes
    /// lazily, so a post-write re-probe is Warn, never Fail).
    pub fn system_reg(&self) -> PathBuf {
        self.prefix.join("system.reg")
    }

    /// `<prefix>/dosdevices/z:` — the drive that makes an out-of-`drive_c` game
    /// reachable.
    pub fn z_drive(&self) -> PathBuf {
        self.prefix.join("dosdevices/z:")
    }

    /// `<prefix>/drive_c/openxr/wineopenxr64.json` — the per-bottle OpenXR manifest.
    pub fn openxr_manifest(&self) -> PathBuf {
        self.prefix.join("drive_c/openxr/wineopenxr64.json")
    }

    /// True when `cxbottle.conf` exists (lib.sh's and doctor's existence test).
    pub fn exists(&self) -> bool {
        self.conf_path().is_file()
    }

    /// `require_bottle` parity: build and validate in one step.
    ///
    /// The error carries doctor's `bottle.exists` message and remedy verbatim, so
    /// a Fatal raised here reads exactly like the doctor row for the same
    /// condition.
    pub fn resolve(name: &str) -> Result<Bottle, SabrageError> {
        let bottle = Bottle::unvalidated(name);
        if bottle.exists() {
            Ok(bottle)
        } else {
            Err(SabrageError::fatal(
                format!(
                    "bottle '{}' not found at {}",
                    bottle.name,
                    bottle.prefix.display()
                ),
                "create it in the CrossOver UI (win11_64)",
            ))
        }
    }
}

/// Resolve the Beat Saber directory the way lib.sh/doctor.sh do:
/// `${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/<bs_dir_leaf>}`.
///
/// Note the deliberate reproduction of a shell quirk: when no bottle is resolved,
/// `$PREFIX` is empty and the fallback becomes the absolute-looking
/// `/drive_c/Program Files (x86)/…`. doctor never *uses* that value (sections 8
/// and 3-z are skipped when there is no bottle and no override), but the string
/// still appears in remedy text, so it must match.
pub fn resolve_bs_dir(bottle: Option<&Bottle>, bs_dir_override: Option<&Path>) -> PathBuf {
    if let Some(dir) = bs_dir_override {
        return dir.to_path_buf();
    }
    let prefix = bottle
        .map(|b| b.prefix.display().to_string())
        .unwrap_or_default();
    PathBuf::from(format!(
        "{prefix}/drive_c/Program Files (x86)/Steam/steamapps/common/{}",
        contract().game.bs_dir_leaf
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_derived_from_the_explicit_root() {
        let p = Paths::new("/repo");
        assert_eq!(p.oxrsys, PathBuf::from("/repo/ext/oxrsys"));
        assert_eq!(p.oxr_build, PathBuf::from("/repo/ext/oxrsys/build-x64"));
        assert_eq!(
            p.oxr_dylib,
            PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib")
        );
        assert_eq!(
            p.oxr_helper_built,
            PathBuf::from("/repo/ext/oxrsys/build-helper-arm64/runtime/oxrsys-encoder-helper")
        );
        assert_eq!(
            p.oxr_helper_staged,
            PathBuf::from("/repo/ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper")
        );
        assert_eq!(
            p.woxr_dll,
            PathBuf::from("/repo/ext/wineopenxr/build/src/pe/wineopenxr.dll")
        );
        assert_eq!(
            p.woxr_so,
            PathBuf::from("/repo/ext/wineopenxr/build/src/unix/wineopenxr.so")
        );
        assert_eq!(
            p.alvr_dashboard,
            PathBuf::from("/repo/ext/ALVR/target/release/alvr_dashboard")
        );
        assert_eq!(
            p.gbe_dll,
            PathBuf::from("/repo/third_party/gbe/steam_api64.dll")
        );
        assert_eq!(
            p.host_xr_json,
            PathBuf::from("/usr/local/share/openxr/1/active_runtime.x86_64.json")
        );
        assert_eq!(p.rel_display(Path::new("/repo/ext/oxrsys")), "ext/oxrsys");
        assert_eq!(p.rel_display(Path::new("/elsewhere")), "/elsewhere");
    }

    #[test]
    fn crossover_helpers_are_none_without_crossover() {
        // Whether CrossOver exists on this machine is machine state; assert the
        // invariant instead: cx/wine/wineserver are Some exactly when cx_app is.
        let p = Paths::new("/repo");
        assert_eq!(p.cx.is_some(), p.cx_app.is_some());
        assert_eq!(p.wine.is_some(), p.cx_app.is_some());
        assert_eq!(p.wineserver.is_some(), p.cx_app.is_some());
        assert_eq!(
            p.cx_dxmt("x86_64-unix/winemetal.so").is_some(),
            p.cx.is_some()
        );
        if let Some(cx) = &p.cx {
            assert!(cx.ends_with("Contents/SharedSupport/CrossOver"));
            assert_ne!(cx, Path::new("/Contents/SharedSupport/CrossOver"));
        }
    }

    #[test]
    fn bottle_paths_match_lib_sh() {
        let b = Bottle::unvalidated("Steam");
        assert!(b
            .prefix
            .ends_with("Library/Application Support/CrossOver/Bottles/Steam"));
        assert!(b.sys32.ends_with("Steam/drive_c/windows/system32"));
        assert!(b.conf_path().ends_with("Steam/cxbottle.conf"));
        assert!(b.z_drive().ends_with("Steam/dosdevices/z:"));
        assert!(b
            .openxr_manifest()
            .ends_with("Steam/drive_c/openxr/wineopenxr64.json"));
    }

    #[test]
    fn bs_dir_override_wins_and_default_uses_the_contract_leaf() {
        let b = Bottle::unvalidated("Steam");
        let over = PathBuf::from("/games/bs");
        assert_eq!(resolve_bs_dir(Some(&b), Some(&over)), over);
        assert_eq!(resolve_bs_dir(None, Some(&over)), over);

        let def = resolve_bs_dir(Some(&b), None);
        assert!(def.starts_with(&b.prefix));
        assert!(def.ends_with("drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294"));

        // No bottle, no override: the empty-$PREFIX shell quirk, reproduced.
        assert_eq!(
            resolve_bs_dir(None, None),
            PathBuf::from("/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294")
        );
    }
}
