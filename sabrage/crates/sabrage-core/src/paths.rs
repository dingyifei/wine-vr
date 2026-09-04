//! The typed port of `scripts/demo/lib.sh`'s derived-path block.
//!
//! Two shell traps are closed at the type level: `cx`/`wine`/`wineserver` are
//! `None` when CrossOver is absent, where lib.sh builds the bogus absolute
//! `/Contents/SharedSupport/CrossOver` from an unset `CX_APP`; `adb` is `None`,
//! where lib.sh leaves `ADB=""` and every call site guards with `[ -n "$ADB" ]`.
//!
//! Sabrage's binary can sit anywhere, so [`Paths`] takes `repo_root` as explicit
//! input (persisted in Sabrage settings). Changing it invalidates install state:
//! the host OpenXR manifest embeds the absolute dylib path under it.

use std::path::{Path, PathBuf};

use crate::contract::contract;
use crate::error::SabrageError;

/// `$HOME`, or `/` if the environment has none.
///
/// Read-only probes only: with an empty `HOME` every derived path comes out
/// *relative* to the working directory, so a stage that writes must go through
/// [`home_dir_checked`] (tests::home_is_required_to_be_absolute_and_non_empty).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// `$HOME`, validated: present, non-empty, and absolute.
///
/// The gate every *mutating* entry point should pass before constructing a
/// [`Paths`] — see [`Paths::new_checked`]. `home_dir`'s fallbacks are fine for
/// a doctor row that will simply report "missing"; they are not fine for
/// `setup`, `install`, `run`, `stop`, or a store write, where a bad `HOME`
/// silently redirects the write out of the user store.
pub fn home_dir_checked() -> Result<PathBuf, SabrageError> {
    check_home(std::env::var_os("HOME"))
}

/// [`home_dir_checked`]'s rule, as a pure function of the raw variable (so it
/// is testable without mutating this process's environment).
fn check_home(raw: Option<std::ffi::OsString>) -> Result<PathBuf, SabrageError> {
    let remedy = "start Sabrage with HOME set to your user home directory";
    let value = raw.filter(|v| !v.is_empty()).ok_or_else(|| {
        SabrageError::fatal(
            "HOME is not set, so ~/Library/Application Support cannot be located",
            remedy,
        )
    })?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(SabrageError::fatal(
            format!("HOME is not an absolute path: {}", path.display()),
            remedy,
        ));
    }
    Ok(path)
}

/// `~/Library/Application Support/CrossOver/Bottles`.
pub fn bottles_root() -> PathBuf {
    home_dir().join("Library/Application Support/CrossOver/Bottles")
}

/// `~/Library/Application Support/Sabrage` — Sabrage's own store.
///
/// GUI-only state lives here and **never** in the repo or in OXRSys's
/// directory (CLAUDE.md, "Sabrage ⇄ demo.sh parity"), so nothing under it is
/// a parity artifact. [`crate::privilege::sabrage_support_dir`] is a thin
/// alias for this.
pub fn sabrage_support_dir() -> PathBuf {
    home_dir().join("Library/Application Support/Sabrage")
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

/// Environment override for the repo root, checked by [`resolve_repo_root`].
pub const REPO_ROOT_ENV: &str = "SABRAGE_REPO_ROOT";

/// The two files whose presence together identify a wine-vr checkout.
///
/// `demo.sh` alone is too weak (any script by that name); the pair is what the
/// dispatcher itself relies on (`source "$ROOT/scripts/demo/lib.sh"`).
pub const REPO_ROOT_MARKERS: [&str; 2] = ["demo.sh", "scripts/demo/lib.sh"];

/// Resolve the wine-vr checkout root: `override_root` when the caller has one
/// (CLI `--repo-root`, the GUI's persisted `settings.repo_root`), else a
/// non-empty `SABRAGE_REPO_ROOT`, else the first ancestor of
/// `std::env::current_exe()` holding both [`REPO_ROOT_MARKERS`].
///
/// The first two are normalized logically ([`logical_absolute`]) to match
/// demo.sh's `ROOT="$(cd "$(dirname "$0")" && pwd)"`: canonicalizing would make
/// the front-ends write different host-manifest bytes and thrash with sudo (A2-6;
/// tests::a_relative_root_becomes_absolute_without_resolving_symlinks).
///
/// An explicit root skips [`REPO_ROOT_MARKERS`] validation so scratch and fixture
/// trees work (tests::explicit_override_wins_and_is_canonicalized).
///
/// # Errors
///
/// Fatal when `current_exe()` fails, or no ancestor holds both markers and no override was given.
pub fn resolve_repo_root(override_root: Option<&str>) -> Result<PathBuf, SabrageError> {
    if let Some(explicit) = override_root.filter(|s| !s.is_empty()) {
        return Ok(logical_absolute(PathBuf::from(explicit)));
    }
    if let Some(from_env) = std::env::var(REPO_ROOT_ENV).ok().filter(|s| !s.is_empty()) {
        return Ok(logical_absolute(PathBuf::from(from_env)));
    }
    let exe = std::env::current_exe().map_err(|e| {
        SabrageError::fatal(
            format!("cannot resolve Sabrage's own executable path: {e}"),
            format!("set {REPO_ROOT_ENV} to the wine-vr checkout"),
        )
    })?;
    find_repo_root_from(&exe).ok_or_else(|| {
        SabrageError::fatal(
            format!(
                "could not locate the wine-vr repo root (looked for demo.sh + \
                 scripts/demo/lib.sh in every directory above {}); set \
                 {REPO_ROOT_ENV} to override",
                exe.display()
            ),
            format!(
                "set {REPO_ROOT_ENV} to the wine-vr checkout, or run Sabrage from a build \
                 under that checkout"
            ),
        )
    })
}

/// The working directory the way `pwd` prints it: `$PWD` when it really names
/// this process's cwd, `getcwd()` otherwise, `None` when neither is available.
///
/// `$PWD` is the *logical* cwd (`pwd -L`, symlinks intact) that demo.sh's
/// `ROOT` is built from; `getcwd()` is the physical one. A GUI-launched `.app`
/// or cargo test binary can inherit a stale `PWD`, hence the dev/ino check.
fn logical_cwd() -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;

    let physical = std::env::current_dir().ok();
    let logical = std::env::var_os("PWD").map(PathBuf::from).filter(|p| {
        p.is_absolute()
            && match (std::fs::metadata(p), std::fs::metadata(".")) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => false,
            }
    });
    logical.or(physical)
}

fn logical_absolute(p: PathBuf) -> PathBuf {
    use std::path::Component;

    let abs = if p.is_absolute() {
        p
    } else {
        match logical_cwd() {
            Some(cwd) => cwd.join(p),
            // No working directory to resolve against: normalize what we have
            // rather than inventing a root.
            None => p,
        }
    };

    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                // The ordinary case: pop the previous name, textually.
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                // `cd /..` is `/` in every shell.
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {}
                // A relative path with no working directory: keep the `..`.
                _ => out.push(Component::ParentDir.as_os_str()),
            },
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        out
    }
}

/// First ancestor of `start` (excluding `start` itself, which is an executable
/// path, not a directory) that contains both [`REPO_ROOT_MARKERS`].
pub fn find_repo_root_from(start: &Path) -> Option<PathBuf> {
    let mut dir = start.parent();
    while let Some(d) = dir {
        if REPO_ROOT_MARKERS.iter().all(|m| d.join(m).is_file()) {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
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

    /// Sabrage's own store, `~/Library/Application Support/Sabrage`
    /// ([`sabrage_support_dir`]). Has no lib.sh counterpart — demo.sh has no
    /// state of its own. A field rather than a call so tests can redirect it
    /// away from the real `$HOME`, exactly as they already do with
    /// `oxr_appsup`.
    pub sabrage_appsup: PathBuf,
}

impl Paths {
    /// Build the path set for `repo_root`, probing the machine for CrossOver and
    /// adb exactly the way lib.sh does.
    ///
    /// Probing is read-only (three `stat`s and a `$PATH` walk).
    pub fn new(repo_root: impl Into<PathBuf>) -> Paths {
        let root: PathBuf = repo_root.into();
        let home = home_dir();

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
            sabrage_appsup: sabrage_support_dir(),
            root,
        }
    }

    /// [`Paths::new`], but fails closed when `$HOME` is unusable
    /// ([`home_dir_checked`]).
    ///
    /// The constructor every *mutating* entry point should use — the CLI's
    /// stage dispatch, the Tauri command layer, and anything that writes into
    /// the Sabrage or OXRSys store. With an empty `HOME`, [`Paths::new`]
    /// derives `Library/Application Support/OXRSys` *relative to the working
    /// directory*, and `setup` would then create it there; this refuses first,
    /// with a remedy, the way `demo.sh`'s `set -u` refuses.
    pub fn new_checked(repo_root: impl Into<PathBuf>) -> Result<Paths, SabrageError> {
        home_dir_checked()?;
        Ok(Paths::new(repo_root))
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

    /// `<root>/logs` — where `run` writes `beatsaber-<ts>.log`.
    ///
    /// Reference: `scripts/demo/run.sh`. Both front-ends write here, so the Logs
    /// screen lists the shell's past runs too. Gitignored on purpose.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// `<sabrage_appsup>/session-state.json` — the crash-recovery record
    /// ([`crate::session::state::SessionState`]).
    ///
    /// Sabrage-only: demo.sh's guards are shell traps, which a `SIGKILL` or a
    /// power loss simply skips, leaving the Mac's audio output on BlackHole
    /// with nothing able to say what it was before. This file is what lets
    /// the next launch (or Stop) put it back.
    pub fn session_state_path(&self) -> PathBuf {
        self.sabrage_appsup.join("session-state.json")
    }

    /// `<oxr_appsup>/.oxrsys-runtime.toml.lock` — the advisory lock that
    /// serializes create-and-edit of [`Paths::toml_path`] between front-ends.
    ///
    /// The config is write-once and hand-editable, so its editor is a
    /// read-modify-write with a backup in the middle; two processes doing that
    /// at once can lose the whole document (`config::runtime_toml`). A
    /// dotfile, so it never shows up next to the config the user opens.
    pub fn toml_lock_path(&self) -> PathBuf {
        self.oxr_appsup.join(".oxrsys-runtime.toml.lock")
    }

    /// `<sabrage_appsup>/session-state.lock` — the advisory lock that
    /// serializes the read-modify-write of [`Paths::session_state_path`]
    /// across processes.
    ///
    /// The record is one application-wide file, but two front-ends (the GUI and
    /// the `sabrage` CLI) can reach it at once; an atomic rename keeps the JSON
    /// from tearing without keeping one writer from losing the other's update.
    /// A separate lock file rather than locking the record itself, so the lock
    /// survives the rename that replaces it.
    pub fn session_state_lock_path(&self) -> PathBuf {
        self.sabrage_appsup.join("session-state.lock")
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
/// With no bottle and no override the shell's empty-`$PREFIX` quirk is
/// reproduced on purpose: the absolute-looking `/drive_c/Program Files
/// (x86)/...` string appears in remedy text and must match
/// (tests::bs_dir_override_wins_and_default_uses_the_contract_leaf).
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
mod tests;
