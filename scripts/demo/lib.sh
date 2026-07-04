# Shared configuration + helpers for demo.sh stages. zsh, sourced (never executed).
# Expects WINEVR_ROOT exported by demo.sh.

ROOT="$WINEVR_ROOT"

# ---- pinned dependency sources -----------------------------------------------
DEPS_URL="https://github.com/dingyifei/wine-vr/releases/download/deps-v1"
DXMT_TGZ_SHA256="487e57e86e9866c922f8d8e42a50cb0818697b927739b6741fae8f4447e2df96"
GBE_DLL_SHA256="cc5a2c9cb93fdbde7dadb825138ab7f694e3f8c310cdd675f733eaa784cbcc3e"

# ---- user-tunable environment ------------------------------------------------
# WINEVR_BOTTLE   (required by doctor/install/run) CrossOver bottle name, e.g. Steam
# WINEVR_BS_DIR   Beat Saber 1.29.4 install dir (DepotDownloader output).
#                 Default (resolved once the bottle is known): the bottle's
#                 standard Steam library — <bottle>/drive_c/Program Files (x86)/
#                 Steam/steamapps/common/Beat Saber 1294

# ---- derived paths -------------------------------------------------------------
for _cx in "$HOME/Applications/CrossOver.app" "/Applications/CrossOver.app"; do
  [ -d "$_cx" ] && CX_APP="$_cx" && break
done
CX="${CX_APP:-}/Contents/SharedSupport/CrossOver"
WINE="$CX/bin/wine"
WINESERVER="$CX/bin/wineserver"

OXR_APPSUP="$HOME/Library/Application Support/OXRSys"
TOML="$OXR_APPSUP/oxrsys-runtime.toml"
HOST_XR_JSON="/usr/local/share/openxr/1/active_runtime.x86_64.json"

OXRSYS="$ROOT/ext/oxrsys"
WOXR="$ROOT/ext/wineopenxr"
ALVR="$ROOT/ext/ALVR"
DXMT_ART="$ROOT/ext/dxmt-artifacts"
GBE_DLL="$ROOT/third_party/gbe/steam_api64.dll"

OXR_BUILD="$OXRSYS/build-x64"
OXR_DYLIB="$OXR_BUILD/runtime/liboxrsys-runtime.dylib"
OXR_ALVR_DYLIB="$OXR_BUILD/runtime/libalvr_server_core.dylib"
OXR_RUNTIME_JSON="$OXR_BUILD/runtime/oxrsys-runtime.json"
WOXR_DLL="$WOXR/build/src/pe/wineopenxr.dll"
WOXR_SO="$WOXR/build/src/unix/wineopenxr.so"

ADB="$HOME/Library/Android/sdk/platform-tools/adb"
command -v "$ADB" >/dev/null 2>&1 || ADB="$(command -v adb 2>/dev/null || true)"

BS_APPID=620980

# ---- output helpers (print -r: never mangle backslashes in windows paths) -------
_G=$'\e[32m'; _Y=$'\e[33m'; _R=$'\e[31m'; _N=$'\e[0m'
FAILCOUNT=0
info() { print -r -- "  $*"; }
ok()   { print -r -- "  ${_G}OK${_N}   $*"; }
warn() { print -r -- "  ${_Y}WARN${_N} $*"; }
fail() { print -r -- "  ${_R}FAIL${_N} $1"; [ $# -gt 1 ] && print -r -- "       remedy: $2"; FAILCOUNT=$((FAILCOUNT+1)); }
die()  { print -r -- "${_R}FATAL${_N} $*" >&2; exit 1; }

bs_version() { # best-effort Beat Saber version: marker file, else the Unity build stamp
  cat "$BS_DIR/BeatSaberVersion.txt" 2>/dev/null && return
  grep -a -o -E -m1 '[0-9]{1,2}\.[0-9]{1,3}\.[0-9]{1,3}_[0-9]{6,}' \
    "$BS_DIR/Beat Saber_Data/globalgamemanagers" 2>/dev/null || echo '?'
}

# ---- shared helpers -------------------------------------------------------------
require_bottle() {
  [ -n "${WINEVR_BOTTLE:-}" ] || die "CrossOver bottle name required: pass --bottle <name> or set WINEVR_BOTTLE.
       Existing bottles: $(ls "$HOME/Library/Application Support/CrossOver/Bottles" 2>/dev/null | tr '\n' ' ')"
  PREFIX="$HOME/Library/Application Support/CrossOver/Bottles/$WINEVR_BOTTLE"
  SYS32="$PREFIX/drive_c/windows/system32"
  [ -f "$PREFIX/cxbottle.conf" ] || die "bottle '$WINEVR_BOTTLE' not found at $PREFIX — create it in CrossOver (win11_64) first"
  # Beat Saber location: --bs-dir/WINEVR_BS_DIR override, else the bottle's
  # standard Steam library path.
  BS_DIR="${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294}"
  DEPOT_CMD="DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 -username <steam-user> -dir \"$BS_DIR\""
}

sha256_ok() { # file expected-hash
  [ -f "$1" ] || return 1
  [ "$(shasum -a 256 "$1" | awk '{print $1}')" = "$2" ]
}

install_if_changed() { # src dst  -> copies only when content differs; prints action
  if cmp -s "$1" "$2" 2>/dev/null; then
    info "unchanged: $2"
  else
    cp "$1" "$2" || die "copy failed: $1 -> $2"
    ok "installed: $2"
  fi
}

win_path() { # unix absolute path -> windows path: C:\ inside the bottle's drive_c, else Z:\ (z: -> /)
  if [ -n "${PREFIX:-}" ] && [[ "$1" == "$PREFIX/drive_c/"* ]]; then
    local rel="${1#$PREFIX/drive_c/}"
    print -- "C:\\${rel//\//\\}"
  else
    print -- "Z:${1//\//\\}"
  fi
}

fetch_pinned() { # url dest expected-sha256 label
  local url="$1" dest="$2" sha="$3" label="$4"
  if sha256_ok "$dest" "$sha"; then info "already present: $label"; return 0; fi
  mkdir -p "$(dirname "$dest")"
  info "downloading $label ..."
  curl -fL --retry 3 --progress-bar -o "$dest.tmp" "$url" || die "download failed: $url"
  sha256_ok "$dest.tmp" "$sha" || die "sha256 mismatch for $label (got $(shasum -a 256 "$dest.tmp" | awk '{print $1}'))"
  mv "$dest.tmp" "$dest"
  ok "fetched $label (sha256 verified)"
}
