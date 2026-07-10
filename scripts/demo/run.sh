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
[ -x "$WINE" ] || die "CrossOver wine not found at $WINE — is CrossOver installed?"
[ -f "$OXR_DYLIB" ] && [ -f "$WOXR_DLL" ] || die "bridge not built — ./demo.sh build"
[ -f "$HOST_XR_JSON" ] || die "host OpenXR registration missing — ./demo.sh install --bottle $WINEVR_BOTTLE"
# bottle + global overlay currency (a fresh bottle or a CrossOver update passes every
# machine-global check yet launches with no VR — catch it here, not as a black window)
cmp -s "$WOXR_DLL" "$SYS32/wineopenxr.dll" || die "bottle wineopenxr.dll stale/missing — ./demo.sh install --bottle $WINEVR_BOTTLE"
[ -f "$PREFIX/drive_c/openxr/wineopenxr64.json" ] || die "bottle OpenXR manifest missing — ./demo.sh install --bottle $WINEVR_BOTTLE"
grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg" 2>/dev/null || \
  die "bottle ActiveRuntime registry key missing — ./demo.sh install --bottle $WINEVR_BOTTLE"
cmp -s "$DXMT_ART/x86_64-windows/d3d11.dll" "$CX/lib/dxmt/x86_64-windows/d3d11.dll" || \
  die "CrossOver DXMT overlay stale (CrossOver update?) — ./demo.sh install --bottle $WINEVR_BOTTLE"
# The bottle's Graphics Backend setting overrides the CX_GRAPHICS_BACKEND env var; the
# CrossOver GUI writes "" (= auto) which no longer selects DXMT — the game then spins
# forever before D3D11 device creation (no DXMT banner, no session, no streamer).
CXCONF="$PREFIX/cxbottle.conf"
if ! grep -q '^"CX_GRAPHICS_BACKEND" = "dxmt"$' "$CXCONF" 2>/dev/null; then
  if grep -q '^"CX_GRAPHICS_BACKEND"' "$CXCONF" 2>/dev/null; then
    sed -i '' 's/^"CX_GRAPHICS_BACKEND" = ".*"$/"CX_GRAPHICS_BACKEND" = "dxmt"/' "$CXCONF" \
      || die "could not force graphics backend to dxmt in $CXCONF"
  elif grep -q '^\[EnvironmentVariables\]$' "$CXCONF" 2>/dev/null; then
    sed -i '' '/^\[EnvironmentVariables\]$/a\
"CX_GRAPHICS_BACKEND" = "dxmt"
' "$CXCONF" || die "could not force graphics backend to dxmt in $CXCONF"
  else
    printf '\n[EnvironmentVariables]\n"CX_GRAPHICS_BACKEND" = "dxmt"\n' >> "$CXCONF" \
      || die "could not force graphics backend to dxmt in $CXCONF"
  fi
  ok "bottle graphics backend forced to dxmt (was auto/other — the CrossOver GUI can reset this)"
fi
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
DASHBOARD_PID=""
stop_dashboard() {
  if [ -n "$DASHBOARD_PID" ] && kill -0 "$DASHBOARD_PID" 2>/dev/null; then
    kill "$DASHBOARD_PID" 2>/dev/null && print "dashboard: closed"
  fi
  DASHBOARD_PID=""
}
# INT/TERM: tear the game down (wineserver -k) and restore audio, then resignal so
# the script exits with the right status. Wine runs as a background job below and
# the script waits on it, so zsh delivers these traps immediately on a signal.
trap 'stop_dashboard; restore_audio' EXIT
trap 'print ""; print -r -- "-- interrupted: stopping wine"; stop_wine; stop_dashboard; restore_audio; trap - INT;  kill -INT  $$' INT
trap 'print -r -- "-- terminated: stopping wine"; stop_wine; stop_dashboard; restore_audio; trap - TERM; kill -TERM $$' TERM
if [ -n "${WINEVR_NO_AUDIO:-}" ]; then
  info "audio routing disabled (--no-audio) — sound stays on the Mac"
elif [ "$PROTOCOL" = "alvr" ] && command -v SwitchAudioSource >/dev/null 2>&1; then
  if SwitchAudioSource -a -t output | grep -qx "BlackHole 2ch"; then
    PREV_AUDIO_OUT="$(SwitchAudioSource -c -t output)"
    if SwitchAudioSource -t output -s "BlackHole 2ch" >/dev/null 2>&1; then
      print -r -- "audio: default output -> BlackHole 2ch (was: $PREV_AUDIO_OUT)"
      # BlackHole applies the macOS device volume to the loopback samples, so
      # anything under 100% reaches the headset attenuated; volume is per-device,
      # so this never touches the speakers we restore on exit.
      osascript -e 'set volume output volume 100' >/dev/null 2>&1 || true
    else
      warn "could not switch output to BlackHole 2ch — audio stays on the Mac"
      PREV_AUDIO_OUT=""
    fi
  else
    warn "BlackHole 2ch not present (brew install blackhole-2ch + reboot) — audio stays on the Mac"
  fi
fi

# ---- ALVR server dashboard ------------------------------------------------------
# The embedded alvr_server_core hosts the dashboard API on 127.0.0.1:8082 inside
# the game process; the stock dashboard polls until it appears, so launching it
# before the game is fine. Closed again by the exit/signal traps above.
if [ -n "${WINEVR_NO_DASHBOARD:-}" ]; then
  info "ALVR dashboard disabled (--no-dashboard)"
elif [ "$PROTOCOL" != "alvr" ]; then
  :
elif [ -x "$ALVR_DASHBOARD_BIN" ]; then
  "$ALVR_DASHBOARD_BIN" >/dev/null 2>&1 &
  DASHBOARD_PID=$!
  print -r -- "dashboard: ALVR server dashboard opening (connects once the game is up)"
else
  warn "alvr_dashboard not built — ./demo.sh build (continuing without the dashboard)"
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
# Quiet by default: the useful lines (oxrsys/ALVR spdlog, Unity) are not wine
# channels. --verbose restores the wine/openxr firehose for debugging.
if [ -n "${WINEVR_VERBOSE:-}" ]; then export WINEDEBUG="${WINEDEBUG:-fixme-all,+openxr}"
else export WINEDEBUG="${WINEDEBUG:--all}"; fi
export SteamAppId=$BS_APPID SteamGameId=$BS_APPID
mkdir -p "$ROOT/logs"
LOG="$ROOT/logs/beatsaber-$(date +%Y%m%d-%H%M%S).log"

print ""
print -r -- "-- launching Beat Saber through the bridge"
print -r -- "   put the headset ON and open the ALVR client; first frame can take ~30s."
print -r -- "   pause in-game = X/A button or the Quest system button"
print -r -- "   (the left-menu-button pause is a Beat Saber/Unity limitation on every OpenXR runtime)"
print -r -- "   stop: Ctrl-C here, or ./demo.sh stop --bottle $WINEVR_BOTTLE from another shell"
print -r -- "   exe: $BS_WIN"
print -r -- "   log: $LOG"
print ""

# Background + wait (instead of a foreground pipeline) so INT/TERM traps run
# immediately; quitting the game from its own menu ends this too.
"$WINE" --bottle "$WINEVR_BOTTLE" --no-update --cx-app "$BS_WIN" > >(tee "$LOG") 2>&1 &
WINE_PID=$!
wait $WINE_PID
rc=$?
print ""
print -r -- "wine exited with status $rc (log: $LOG)"
exit $rc
