# demo.sh doctor — check every prerequisite with a one-line remedy per failure.
# Sourced by demo.sh after lib.sh. Read-only. Exit code = number of FAILs (0 = ready).
#
# Parity instrumentation: every row is a `chk <status> <slug> …` call (or an explicit
# `tap <slug> <status>` when a row is silent — skipped sections, silent-when-clean
# checks). Slugs are shared with the native Sabrage pipeline's check registry and MUST
# stay literal in this file (the parity tests token-scan it). Human output is
# byte-identical to the pre-instrumentation script; the tap channel only activates
# when WINEVR_DOCTOR_TAP names a file (like WINEVR_DOCTOR_SOFT, opt-in).

print "== wine-vr demo doctor =="

# 0. contract sync — the generated pins file must match contract/ (recipe pinned:
# cat pipeline.toml + both templates, in that order, bytes as-is | shasum -a 256).
# This catches "edited contract/, forgot to regen" with zero Rust available.
_want="$(cat "$ROOT/contract/pipeline.toml" "$ROOT/contract/oxrsys-runtime.toml.template" "$ROOT/contract/active_runtime.x86_64.json.template" 2>/dev/null | shasum -a 256 | awk '{print $1}')"
_have="$(sed -n 's/^# contract-sha256: //p' "$ROOT/scripts/demo/contract.gen.sh" 2>/dev/null | head -1)"
if [ -n "$_have" ] && [ "$_want" = "$_have" ]; then chk ok meta.contract-sync "contract/ in sync with scripts/demo/contract.gen.sh"
else chk fail meta.contract-sync "contract/ and scripts/demo/contract.gen.sh out of sync (contract edited without regen, or the generated file was hand-edited)" "scripts/dev/parity.sh --regen"; fi

# 1. hardware / OS
if [ "$(uname -m)" = "arm64" ]; then chk ok sys.arch "Apple Silicon ($(sysctl -n machdep.cpu.brand_string 2>/dev/null))"
else chk fail sys.arch "not an Apple Silicon Mac ($(uname -m))" "this demo requires an arm64 Mac"; fi
# BGRA-direct encoding (oxrsys bgra-direct, no NV12 pre-convert) needs VT's fixed
# internal RGB->YCbCr under Rosetta — macOS 27+ only; older VT emits all-zero chroma.
OSVER="$(sw_vers -productVersion 2>/dev/null || echo 0)"
if printf '%s\n27\n' "${OSVER%%.*}" | sort -n | tail -1 | grep -qx "${OSVER%%.*}"; then
  chk ok sys.macos27 "macOS $OSVER (>= 27: VT encodes BGRA directly under Rosetta)"
else chk fail sys.macos27 "macOS $OSVER < 27 — in-process BGRA-direct encode produces green video (VT zero-chroma bug); the in-process fallback needs macOS 27+ even with the native helper (native helper path unaffected)" "upgrade to macOS 27+, or pin ext/oxrsys back to the NV12-era revision cf5f926"; fi

# 2. CrossOver
if [ -n "${CX_APP:-}" ]; then
  tap cx.present ok
  CXVER="$(defaults read "$CX_APP/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo 0)"
  if printf '%s\n26.2\n' "$CXVER" | sort -V | tail -1 | grep -qx "$CXVER"; then
    chk ok cx.version "CrossOver $CXVER at $CX_APP"
  else chk fail cx.version "CrossOver $CXVER < 26.2" "upgrade CrossOver to 26.2+"; fi
else
  chk fail cx.present "CrossOver.app not found" "install CrossOver into ~/Applications or /Applications"
  tap cx.version skipped
fi

# 3. bottle (soft: a missing bottle FAILs but the machine-side checks still run)
BOTTLE_OK=0
if [ -z "${WINEVR_BOTTLE:-}" ]; then
  chk fail bottle.named "no bottle name given (--bottle/WINEVR_BOTTLE)" "create a win11_64 bottle in CrossOver; existing: $(ls "$HOME/Library/Application Support/CrossOver/Bottles" 2>/dev/null | tr '\n' ' ')"
  tap bottle.exists skipped; tap bottle.template skipped; tap bottle.gfx-dxmt skipped
else
  tap bottle.named ok
  PREFIX="$HOME/Library/Application Support/CrossOver/Bottles/$WINEVR_BOTTLE"
  SYS32="$PREFIX/drive_c/windows/system32"
  if [ -f "$PREFIX/cxbottle.conf" ]; then
    BOTTLE_OK=1
    chk ok bottle.exists "bottle '$WINEVR_BOTTLE' exists"
    if grep -q '^"Template" = "win11_64"' "$PREFIX/cxbottle.conf" 2>/dev/null; then chk ok bottle.template "bottle template win11_64"
    else chk warn bottle.template "bottle template is not win11_64 ($(grep '^"Template"' "$PREFIX/cxbottle.conf" 2>/dev/null | head -1)) — only win11_64 is verified"; fi
    if grep -q '^"CX_GRAPHICS_BACKEND" = "dxmt"$' "$PREFIX/cxbottle.conf" 2>/dev/null; then chk ok bottle.gfx-dxmt "bottle graphics backend = dxmt"
    else chk fail bottle.gfx-dxmt "bottle graphics backend is not dxmt (the CrossOver GUI 'auto' setting no longer selects DXMT — game stalls before D3D11 init, streamer never starts)" "./demo.sh run auto-fixes this, or set Graphics Backend to DXMT in the CrossOver bottle settings"; fi
  else
    chk fail bottle.exists "bottle '$WINEVR_BOTTLE' not found at $PREFIX" "create it in the CrossOver UI (win11_64)"
    tap bottle.template skipped; tap bottle.gfx-dxmt skipped
  fi
fi
WINEVR_BOTTLE="${WINEVR_BOTTLE:-<name>}"   # placeholder keeps remedy strings valid under set -u
PREFIX="${PREFIX:-}"; SYS32="${SYS32:-}"
BS_DIR="${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/$BS_DIR_LEAF}"
DEPOT_CMD="DepotDownloader -app $BS_APPID -depot $BS_DEPOT -manifest $BS_MANIFEST -username <steam-user> -dir \"$BS_DIR\""
if [ "$BOTTLE_OK" = 1 ] && [[ "$BS_DIR" != "$PREFIX/drive_c/"* ]]; then
  if [ -e "$PREFIX/dosdevices/z:" ]; then chk ok bottle.zdrive "bottle z: drive maps / (Beat Saber lives outside drive_c)"
  else chk fail bottle.zdrive "Beat Saber is outside drive_c but the bottle has no z: drive" "add dosdevices/z: -> / or move the install under drive_c"; fi
else
  tap bottle.zdrive skipped
fi

# 4. toolchain
for _t in tool.cmake:cmake tool.ninja:ninja tool.git:git tool.curl:curl tool.mingw:x86_64-w64-mingw32-gcc; do
  if command -v ${_t#*:} >/dev/null 2>&1; then chk ok ${_t%%:*} "${_t#*:}"
  else chk fail ${_t%%:*} "${_t#*:} missing" "brew install cmake ninja git mingw-w64"; fi
done

# 5. rust (AlvrServerCore.cmake requires a rustup toolchain with the x86_64 target)
if command -v rustup >/dev/null 2>&1 && rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin; then
  chk ok rust.x64-target "rustup with x86_64-apple-darwin target"
else chk fail rust.x64-target "rustup x86_64-apple-darwin target missing" "install rustup via https://rustup.rs and source ~/.cargo/env (brew's rustup is keg-only/not on PATH), then: rustup toolchain install stable && rustup target add x86_64-apple-darwin"; fi

# 6. submodules
for _s in src.oxrsys:"$OXRSYS" src.wineopenxr:"$WOXR" src.alvr:"$ALVR"; do
  _sm="${_s#*:}"
  if [ -f "$_sm/.git" ] || [ -d "$_sm/.git" ]; then chk ok "${_s%%:*}" "submodule $(basename $_sm) present"
  else chk fail "${_s%%:*}" "submodule $(basename $_sm) not initialized" "./demo.sh setup"; fi
done
if grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs" 2>/dev/null; then
  chk ok src.alvr-patchset "ALVR oxrsys patch set present"
else chk fail src.alvr-patchset "ALVR submodule missing the oxrsys patches" "./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)"; fi

# 7. pinned binaries
if dxmt_files_ok; then
  if dxmt_ok; then chk ok dep.dxmt "dxmt-artifacts (monofunc fork) present, provenance verified"
  else chk warn dep.dxmt "dxmt-artifacts present but provenance marker missing/stale — ./demo.sh setup re-fetches the pinned set"; fi
else chk fail dep.dxmt "ext/dxmt-artifacts missing or incomplete" "./demo.sh setup"; fi
if sha256_ok "$GBE_DLL" "$GBE_DLL_SHA256"; then chk ok dep.goldberg "Goldberg steam_api64.dll (sha256 verified)"
elif [ -f "$GBE_DLL" ]; then chk warn dep.goldberg "Goldberg dll present but hash differs from the pinned build"
else chk fail dep.goldberg "Goldberg dll missing" "./demo.sh setup"; fi

# 8. Beat Saber 1.29.4
if [ "$BOTTLE_OK" = 0 ] && [ -z "${WINEVR_BS_DIR:-}" ]; then
  info "Beat Saber check skipped (needs --bottle or --bs-dir)"
  tap game.present skipped; tap game.version skipped
elif [ -f "$BS_DIR/Beat Saber.exe" ]; then
  tap game.present ok
  BSVER="$(bs_version)"
  case "$BSVER" in
    1.29.4*) chk ok game.version "Beat Saber $BSVER at $BS_DIR" ;;
    *) chk warn game.version "Beat Saber version '$BSVER' is not 1.29.4 — the Meta account gate may block it" ;;
  esac
else
  chk fail game.present "Beat Saber 1.29.4 not found at $BS_DIR" "$DEPOT_CMD  (or set WINEVR_BS_DIR)"
  tap game.version skipped
fi

# 9. build outputs
for _b in build.oxr-dylib:"$OXR_DYLIB" build.alvr-core:"$OXR_ALVR_DYLIB" build.runtime-json:"$OXR_RUNTIME_JSON" build.woxr-dll:"$WOXR_DLL" build.woxr-so:"$WOXR_SO" build.dashboard:"$ALVR_DASHBOARD_BIN"; do
  _f="${_b#*:}"
  if [ -f "$_f" ]; then chk ok "${_b%%:*}" "built: ${_f#$ROOT/}"
  else chk fail "${_b%%:*}" "missing build output: ${_f#$ROOT/}" "./demo.sh build"; fi
done
# 9b. native-arm64 encoder helper (staged next to the runtime dylib — the runtime
# locates it beside its own dylib; helper has no probe flag, so checks stop at arch)
if [ -f "$OXR_HELPER_BIN" ]; then
  chk ok build.helper-staged "built: ${OXR_HELPER_BIN#$ROOT/} (staged next to the runtime dylib)"
  if helper_is_arm64 "$OXR_HELPER_BIN"; then chk ok build.helper-arm64 "encoder helper is arm64"
  else chk fail build.helper-arm64 "encoder helper is not an arm64 executable ($(lipo -archs "$OXR_HELPER_BIN" 2>/dev/null)) — a stale/wrong-arch binary here shadows the staged one" "./demo.sh build (restages the arm64 helper)"; fi
else
  chk fail build.helper-staged "encoder helper not staged: ${OXR_HELPER_BIN#$ROOT/}" "./demo.sh build"
  tap build.helper-arm64 skipped
fi

# 10. global bridge overlay (a CrossOver update silently reverts these)
if [ -n "${CX_APP:-}" ]; then
  for _o in \
    overlay.dxmt-d3d11:"$DXMT_ART/x86_64-windows/d3d11.dll:$CX/lib/dxmt/x86_64-windows/d3d11.dll" \
    overlay.dxmt-winemetal:"$DXMT_ART/x86_64-unix/winemetal.so:$CX/lib/dxmt/x86_64-unix/winemetal.so" \
    overlay.woxr-dll:"$WOXR_DLL:$CX/lib/wine/x86_64-windows/wineopenxr.dll" \
    overlay.woxr-so:"$WOXR_SO:$CX/lib/wine/x86_64-unix/wineopenxr.so"; do
    _slug="${_o%%:*}"; _pair="${_o#*:}"; _src="${_pair%%:*}"; _dst="${_pair#*:}"
    if cmp -s "$_src" "$_dst" 2>/dev/null; then chk ok "$_slug" "global overlay current: $(basename "$_dst")"
    else chk fail "$_slug" "global overlay stale/missing: $(basename "$_dst")" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  done
else
  tap overlay.dxmt-d3d11 skipped; tap overlay.dxmt-winemetal skipped
  tap overlay.woxr-dll skipped; tap overlay.woxr-so skipped
fi

# 11. per-bottle bridge
if [ "$BOTTLE_OK" = 0 ]; then
  info "per-bottle bridge checks skipped (no bottle)"
  tap bottle.woxr-dll skipped; tap bottle.manifest skipped; tap bottle.registry skipped
else
  if cmp -s "$WOXR_DLL" "$SYS32/wineopenxr.dll" 2>/dev/null; then chk ok bottle.woxr-dll "bottle system32/wineopenxr.dll current"
  else chk fail bottle.woxr-dll "bottle wineopenxr.dll stale/missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  if [ -f "$PREFIX/drive_c/openxr/wineopenxr64.json" ]; then chk ok bottle.manifest 'bottle C:\openxr\wineopenxr64.json'
  else chk fail bottle.manifest "bottle OpenXR manifest missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  if grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg" 2>/dev/null; then
    chk ok bottle.registry "bottle registry ActiveRuntime set"
  else chk fail bottle.registry "bottle ActiveRuntime registry key missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
fi

# 12. host loader registration (wine secure-exec ignores XR_RUNTIME_JSON; this file is load-bearing)
if [ -f "$HOST_XR_JSON" ]; then
  LP="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["runtime"]["library_path"])' "$HOST_XR_JSON" 2>/dev/null)"
  PYRC=$?
  if [ $PYRC -ne 0 ]; then
    chk fail host.manifest "cannot parse $HOST_XR_JSON (broken python3 or malformed JSON)" "check 'python3 -V' works (xcode-select --install), then inspect the file"
  elif [ "$LP" = "$OXR_DYLIB" ] && [ -f "$LP" ]; then chk ok host.manifest "host OpenXR registration -> $LP"
  elif [ -n "$LP" ] && [ -f "$LP" ]; then chk warn host.manifest "host registration points at $LP (expected $OXR_DYLIB)"
  else chk fail host.manifest "host registration points at a missing dylib" "./demo.sh install --bottle $WINEVR_BOTTLE (sudo rewrites $HOST_XR_JSON)"; fi
else chk fail host.manifest "$HOST_XR_JSON missing" "./demo.sh install --bottle $WINEVR_BOTTLE (sudo writes it)"; fi

# 13. runtime config
if [ -f "$TOML" ]; then
  # last assignment wins, like the runtime's parser (Config.cpp)
  PROTO="$(awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{v=$2} END{print v}' "$TOML")"
  if [ "$PROTO" = "alvr" ]; then
    chk ok cfg.protocol.supported "oxrsys-runtime.toml: protocol=alvr"
    tap cfg.protocol.legacy-oxrsys ok
  elif [ "$PROTO" = "oxrsys" ]; then
    tap cfg.protocol.supported ok
    chk fail cfg.protocol.legacy-oxrsys "oxrsys-runtime.toml protocol='"$PROTO"' — the demo streams via ALVR" "set protocol = \"alvr\" in $TOML"
  else
    chk fail cfg.protocol.supported "oxrsys-runtime.toml protocol='"$PROTO"' — the demo streams via ALVR" "set protocol = \"alvr\" in $TOML"
    tap cfg.protocol.legacy-oxrsys skipped
  fi
else
  chk fail cfg.protocol.supported "$TOML missing" "./demo.sh setup"
  tap cfg.protocol.legacy-oxrsys skipped
fi
# 13b. stale client pins in the ALVR session state (machine-local; from past debugging)
SESSJSON="$OXR_APPSUP/alvr/session.json"
if [ -f "$SESSJSON" ]; then
  PINNED="$(python3 -c '
import json,sys
try: s = json.load(open(sys.argv[1]))
except Exception: sys.exit(0)
for n, c in (s.get("client_connections") or {}).items():
    ips = c.get("manual_ips") or []
    if ips: print(n + "=" + ",".join(ips))' "$SESSJSON" 2>/dev/null)"
  PYRC=$?
  PINNED="$(print -r -- "$PINNED" | tr '\n' ' ' | sed 's/^ *$//')"
  if [ $PYRC -ne 0 ]; then chk warn cfg.session-pins "could not inspect $SESSJSON (broken python3?)"
  elif [ -n "$PINNED" ]; then
    chk warn cfg.session-pins "session.json pins client IP(s): $PINNED— fine while the Quest keeps that IP; if streaming stops after a DHCP change, edit the pinned IP in '$SESSJSON' in place (do not delete the file: a recreated session.json streams a black 800x900 screen)"
  else chk ok cfg.session-pins "ALVR session state has no stale manual-IP pins"
  fi
else
  tap cfg.session-pins skipped
fi

# 14. headset-side (warnings only; WiFi streaming needs no USB)
if [ -n "$ADB" ] && "$ADB" devices 2>/dev/null | awk 'NR>1 && $2=="device"' | grep -q .; then
  SER="$("$ADB" devices | awk 'NR>1 && $2=="device"{print $1; exit}')"
  chk ok hs.adb "Quest connected via adb ($SER)"
  if "$ADB" -s "$SER" shell pm list packages 2>/dev/null | grep -q alvr; then chk ok hs.client "ALVR client installed on the Quest"
  else chk warn hs.client "ALVR client not detected on the Quest — install ALVR v20.14.1 client APK"; fi
else
  chk warn hs.adb "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)"
  tap hs.client skipped
fi

# 15. audio loopback (optional)
if command -v SwitchAudioSource >/dev/null 2>&1; then
  if SwitchAudioSource -a -t output 2>/dev/null | grep -qx "BlackHole 2ch"; then chk ok audio.loopback "BlackHole 2ch + switchaudio-osx"
  else chk warn audio.loopback "BlackHole 2ch not present — no in-headset audio (brew install blackhole-2ch, then reboot)"; fi
else chk warn audio.loopback "switchaudio-osx not installed — audio stays on the Mac (brew install switchaudio-osx blackhole-2ch)"; fi

# 16. stale streaming listeners
STALE="$(lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk 'NR>1{print $1"("$2")"}' | sort -u | tr '\n' ' ')"
if [ -n "$STALE" ]; then chk warn net.ports "ports 9943/9944 busy: $STALE— a previous session may still be running"
else chk ok net.ports "streaming ports free"; fi

# 16b. stale adb forwards (legit only for a --wired launch; left behind they squat
# the streaming ports and break Quest WiFi discovery)
if [ -n "$ADB" ]; then
  FWD="$("$ADB" forward --list 2>/dev/null | awk '{print $2}')"
  if print -r -- "$FWD" | grep -qx 'tcp:9943' || print -r -- "$FWD" | grep -qx 'tcp:9944'; then
    chk warn net.adb-forwards "adb forward tcp:9943/tcp:9944 present — expected only for a wired launch (--wired); stale forwards break WiFi discovery — remedy: adb forward --remove tcp:9943 (and tcp:9944), or just a normal ./demo.sh run"
  else
    tap net.adb-forwards ok
  fi
else
  tap net.adb-forwards skipped
fi

print ""
if [ "$FAILCOUNT" -eq 0 ]; then print -r -- "doctor: ${_G}all checks passed${_N} — ./demo.sh run --bottle $WINEVR_BOTTLE"
else print -r -- "doctor: ${_R}$FAILCOUNT check(s) failed${_N} — remedies above"; fi
[ -n "${WINEVR_DOCTOR_SOFT:-}" ] || exit "$FAILCOUNT"
