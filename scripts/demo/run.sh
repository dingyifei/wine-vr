# demo.sh run — launch Beat Saber 1.29.4 through the bridge (the repeatable stage).
# Sourced by demo.sh after lib.sh.
set -o pipefail

print "== wine-vr demo run =="
require_bottle

# ---- preflight (fail fast with remedies instead of a black window) -------------
[ -f "$BS_DIR/Beat Saber.exe" ] || die "Beat Saber not found at $BS_DIR
       download 1.29.4: $DEPOT_CMD
       (or pass --bs-dir / set WINEVR_BS_DIR)"
BSVER="$(bs_version)"
case "$BSVER" in 1.29.4*) : ;; *) warn "Beat Saber version '$BSVER' != 1.29.4 — the Meta gate may block startup" ;; esac
[ -f "$OXR_DYLIB" ] && [ -f "$WOXR_DLL" ] || die "bridge not built — ./demo.sh build"
[ -f "$HOST_XR_JSON" ] || die "host OpenXR registration missing — ./demo.sh install --bottle $WINEVR_BOTTLE"
sha256_ok "$GBE_DLL" "$GBE_DLL_SHA256" || [ -f "$GBE_DLL" ] || die "Goldberg dll missing — ./demo.sh setup"
[ -f "$TOML" ] || die "$TOML missing — ./demo.sh setup"
PROTOCOL="$(awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{print $2; exit}' "$TOML")"
case "$PROTOCOL" in
  alvr) : ;;
  oxrsys) warn "protocol=oxrsys (legacy USB path) — the demo path is alvr" ;;
  *) die "oxrsys-runtime.toml protocol='$PROTOCOL' is not valid for the demo
       set protocol = \"alvr\" in $TOML (or delete the file and re-run ./demo.sh setup)" ;;
esac

# ---- reset the bottle's wineserver (stale servers + steam locks hang startup) ---
print -r -- "-- resetting wineserver for bottle '$WINEVR_BOTTLE'"
WINEPREFIX="$PREFIX" "$WINESERVER" -k 2>/dev/null || true
( WINEPREFIX="$PREFIX" "$WINESERVER" -w 2>/dev/null ) &
_wpid=$!
for _i in {1..50}; do kill -0 $_wpid 2>/dev/null || break; sleep 0.1; done
if kill -0 $_wpid 2>/dev/null; then
  kill $_wpid 2>/dev/null
  warn "wineserver still alive after 5s: $(pgrep -lf wineserver | tr '\n' ' ')"
  die "kill the listed wineserver(s) manually, then re-run"
fi
ok "wineserver down"

# ---- Goldberg steam emulator (no real Steam at runtime; avoids the Meta gate) ---
print -r -- "-- Goldberg"
API="$BS_DIR/Beat Saber_Data/Plugins/x86_64/steam_api64.dll"
[ -f "$API" ] || API="$BS_DIR/steam_api64.dll"
[ -f "$API" ] || die "steam_api64.dll not found under $BS_DIR — is this a complete Beat Saber install?"
APIDIR="$(dirname "$API")"
if [ ! -f "$API.orig-steam" ]; then cp "$API" "$API.orig-steam" || die "backup of original steam_api64.dll failed"; fi
if cmp -s "$GBE_DLL" "$API"; then info "goldberg already installed"
else cp "$GBE_DLL" "$API" || die "goldberg install failed"; ok "installed goldberg -> $API"; fi
printf '%s' "$BS_APPID" > "$APIDIR/steam_appid.txt" || die "writing steam_appid.txt failed"
GSET="$APIDIR/steam_settings"; mkdir -p "$GSET"
: > "$GSET/offline.txt"; : > "$GSET/disable_networking.txt"; : > "$GSET/disable_overlay.txt"

# ---- audio: route the Mac output into BlackHole so ALVR streams it ---------------
PREV_AUDIO_OUT=""
restore_audio() {
  if [ -n "$PREV_AUDIO_OUT" ]; then
    SwitchAudioSource -t output -s "$PREV_AUDIO_OUT" >/dev/null 2>&1 && \
      print "audio: restored output -> $PREV_AUDIO_OUT"
    PREV_AUDIO_OUT=""
  fi
}
trap 'restore_audio' EXIT INT TERM
if [ "$PROTOCOL" = "alvr" ] && command -v SwitchAudioSource >/dev/null 2>&1; then
  if SwitchAudioSource -a -t output | grep -q "BlackHole"; then
    PREV_AUDIO_OUT="$(SwitchAudioSource -c -t output)"
    SwitchAudioSource -t output -s "BlackHole 2ch" >/dev/null && \
      print "audio: default output -> BlackHole 2ch (was: $PREV_AUDIO_OUT)"
  else
    warn "BlackHole 2ch not present (brew install blackhole-2ch + reboot) — audio stays on the Mac"
  fi
fi

# ---- headset client -----------------------------------------------------------
SER=""
[ -n "$ADB" ] && SER="$("$ADB" devices 2>/dev/null | awk 'NR>1 && $2=="device"{print $1; exit}')"
if [ "$PROTOCOL" = "alvr" ]; then
  if [ -n "$SER" ]; then
    # oxrsys-era reverse tunnels squat the ALVR client's stream port (EADDRINUSE)
    "$ADB" -s "$SER" reverse --remove-all 2>/dev/null || true
    info "Quest $SER: cleared adb reverse tunnels (ALVR manages its own)"
  fi
else
  [ -n "$SER" ] || warn "no Quest over adb — the legacy oxrsys protocol needs USB"
  if [ -n "$SER" ]; then
    "$ADB" -s "$SER" reverse --remove-all 2>/dev/null || true
    for p in 9944 9945 9946 9948; do "$ADB" -s "$SER" reverse tcp:$p tcp:$p >/dev/null; done
    "$ADB" -s "$SER" shell am start -n net.demonixis.oxrsys.android/com.oculus.NativeActivity >/dev/null 2>&1
    info "Quest $SER: reverse tunnels up, oxrsys client starting"
  fi
fi

# ---- launch ---------------------------------------------------------------------
BS_WIN="$(win_path "$BS_DIR/Beat Saber.exe")"
export XR_RUNTIME_JSON="$OXR_RUNTIME_JSON"
export CX_GRAPHICS_BACKEND=dxmt
export WINEDEBUG="${WINEDEBUG:-fixme-all,+openxr}"
export SteamAppId=$BS_APPID SteamGameId=$BS_APPID
mkdir -p "$ROOT/logs"
LOG="$ROOT/logs/beatsaber-$(date +%Y%m%d-%H%M%S).log"

print ""
print -r -- "-- launching Beat Saber through the bridge"
print -r -- "   put the headset ON and open the ALVR client; first frame can take ~30s."
print -r -- "   pause in-game = X/A button or the Quest system button"
print -r -- "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)"
print -r -- "   exe: $BS_WIN"
print -r -- "   log: $LOG"
print ""

"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN" 2>&1 | tee "$LOG"
rc=${pipestatus[1]}
print "\nwine exited with status $rc (log: $LOG)"
exit $rc
