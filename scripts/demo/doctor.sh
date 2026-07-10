# demo.sh doctor — check every prerequisite with a one-line remedy per failure.
# Sourced by demo.sh after lib.sh. Read-only. Exit code = number of FAILs (0 = ready).

print "== wine-vr demo doctor =="

# 1. hardware / OS
if [ "$(uname -m)" = "arm64" ]; then ok "Apple Silicon ($(sysctl -n machdep.cpu.brand_string 2>/dev/null))"
else fail "not an Apple Silicon Mac ($(uname -m))" "this demo requires an arm64 Mac"; fi

# 2. CrossOver
if [ -n "${CX_APP:-}" ]; then
  CXVER="$(defaults read "$CX_APP/Contents/Info.plist" CFBundleShortVersionString 2>/dev/null || echo 0)"
  if printf '%s\n26.2\n' "$CXVER" | sort -V | tail -1 | grep -qx "$CXVER"; then
    ok "CrossOver $CXVER at $CX_APP"
  else fail "CrossOver $CXVER < 26.2" "upgrade CrossOver to 26.2+"; fi
else fail "CrossOver.app not found" "install CrossOver into ~/Applications or /Applications"; fi

# 3. bottle (soft: a missing bottle FAILs but the machine-side checks still run)
BOTTLE_OK=0
if [ -z "${WINEVR_BOTTLE:-}" ]; then
  fail "no bottle name given (--bottle/WINEVR_BOTTLE)" "create a win11_64 bottle in CrossOver; existing: $(ls "$HOME/Library/Application Support/CrossOver/Bottles" 2>/dev/null | tr '\n' ' ')"
else
  PREFIX="$HOME/Library/Application Support/CrossOver/Bottles/$WINEVR_BOTTLE"
  SYS32="$PREFIX/drive_c/windows/system32"
  if [ -f "$PREFIX/cxbottle.conf" ]; then
    BOTTLE_OK=1
    ok "bottle '$WINEVR_BOTTLE' exists"
    if grep -q '^"Template" = "win11_64"' "$PREFIX/cxbottle.conf" 2>/dev/null; then ok "bottle template win11_64"
    else warn "bottle template is not win11_64 ($(grep '^"Template"' "$PREFIX/cxbottle.conf" 2>/dev/null | head -1)) — only win11_64 is verified"; fi
  else
    fail "bottle '$WINEVR_BOTTLE' not found at $PREFIX" "create it in the CrossOver UI (win11_64)"
  fi
fi
WINEVR_BOTTLE="${WINEVR_BOTTLE:-<name>}"   # placeholder keeps remedy strings valid under set -u
PREFIX="${PREFIX:-}"; SYS32="${SYS32:-}"
BS_DIR="${WINEVR_BS_DIR:-$PREFIX/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294}"
DEPOT_CMD="DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 -username <steam-user> -dir \"$BS_DIR\""
if [ "$BOTTLE_OK" = 1 ] && [[ "$BS_DIR" != "$PREFIX/drive_c/"* ]]; then
  if [ -e "$PREFIX/dosdevices/z:" ]; then ok "bottle z: drive maps / (Beat Saber lives outside drive_c)"
  else fail "Beat Saber is outside drive_c but the bottle has no z: drive" "add dosdevices/z: -> / or move the install under drive_c"; fi
fi

# 4. toolchain
for tool in cmake ninja git curl x86_64-w64-mingw32-gcc; do
  if command -v $tool >/dev/null 2>&1; then ok "$tool"
  else fail "$tool missing" "brew install cmake ninja git mingw-w64"; fi
done

# 5. rust (AlvrServerCore.cmake requires a rustup toolchain with the x86_64 target)
if command -v rustup >/dev/null 2>&1 && rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin; then
  ok "rustup with x86_64-apple-darwin target"
else fail "rustup x86_64-apple-darwin target missing" "install rustup via https://rustup.rs and source ~/.cargo/env (brew's rustup is keg-only/not on PATH), then: rustup toolchain install stable && rustup target add x86_64-apple-darwin"; fi

# 6. submodules
for sm in "$OXRSYS" "$WOXR" "$ALVR"; do
  if [ -f "$sm/.git" ] || [ -d "$sm/.git" ]; then ok "submodule $(basename $sm) present"
  else fail "submodule $(basename $sm) not initialized" "./demo.sh setup"; fi
done
if grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs" 2>/dev/null; then
  ok "ALVR oxrsys patch set present"
else fail "ALVR submodule missing the oxrsys patches" "./demo.sh setup (checks out the pinned oxrsys-v20.14.1 branch)"; fi

# 7. pinned binaries
if dxmt_files_ok; then
  if dxmt_ok; then ok "dxmt-artifacts (monofunc fork) present, provenance verified"
  else warn "dxmt-artifacts present but provenance marker missing/stale — ./demo.sh setup re-fetches the pinned set"; fi
else fail "ext/dxmt-artifacts missing or incomplete" "./demo.sh setup"; fi
if sha256_ok "$GBE_DLL" "$GBE_DLL_SHA256"; then ok "Goldberg steam_api64.dll (sha256 verified)"
elif [ -f "$GBE_DLL" ]; then warn "Goldberg dll present but hash differs from the pinned build"
else fail "Goldberg dll missing" "./demo.sh setup"; fi

# 8. Beat Saber 1.29.4
if [ "$BOTTLE_OK" = 0 ] && [ -z "${WINEVR_BS_DIR:-}" ]; then
  info "Beat Saber check skipped (needs --bottle or --bs-dir)"
elif [ -f "$BS_DIR/Beat Saber.exe" ]; then
  BSVER="$(bs_version)"
  case "$BSVER" in
    1.29.4*) ok "Beat Saber $BSVER at $BS_DIR" ;;
    *) warn "Beat Saber version '$BSVER' is not 1.29.4 — the Meta account gate may block it" ;;
  esac
else fail "Beat Saber 1.29.4 not found at $BS_DIR" "$DEPOT_CMD  (or set WINEVR_BS_DIR)"; fi

# 9. build outputs
for f in "$OXR_DYLIB" "$OXR_ALVR_DYLIB" "$OXR_RUNTIME_JSON" "$WOXR_DLL" "$WOXR_SO" "$ALVR_DASHBOARD_BIN"; do
  if [ -f "$f" ]; then ok "built: ${f#$ROOT/}"
  else fail "missing build output: ${f#$ROOT/}" "./demo.sh build"; fi
done

# 10. global bridge overlay (a CrossOver update silently reverts these)
if [ -n "${CX_APP:-}" ]; then
  for pair in \
    "$DXMT_ART/x86_64-windows/d3d11.dll:$CX/lib/dxmt/x86_64-windows/d3d11.dll" \
    "$DXMT_ART/x86_64-unix/winemetal.so:$CX/lib/dxmt/x86_64-unix/winemetal.so" \
    "$WOXR_DLL:$CX/lib/wine/x86_64-windows/wineopenxr.dll" \
    "$WOXR_SO:$CX/lib/wine/x86_64-unix/wineopenxr.so"; do
    src="${pair%%:*}"; dst="${pair#*:}"
    if cmp -s "$src" "$dst" 2>/dev/null; then ok "global overlay current: $(basename $dst)"
    else fail "global overlay stale/missing: $(basename $dst)" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  done
fi

# 11. per-bottle bridge
if [ "$BOTTLE_OK" = 0 ]; then info "per-bottle bridge checks skipped (no bottle)"
else
  if cmp -s "$WOXR_DLL" "$SYS32/wineopenxr.dll" 2>/dev/null; then ok "bottle system32/wineopenxr.dll current"
  else fail "bottle wineopenxr.dll stale/missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  if [ -f "$PREFIX/drive_c/openxr/wineopenxr64.json" ]; then ok 'bottle C:\openxr\wineopenxr64.json'
  else fail "bottle OpenXR manifest missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
  if grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg" 2>/dev/null; then
    ok "bottle registry ActiveRuntime set"
  else fail "bottle ActiveRuntime registry key missing" "./demo.sh install --bottle $WINEVR_BOTTLE"; fi
fi

# 12. host loader registration (wine secure-exec ignores XR_RUNTIME_JSON; this file is load-bearing)
if [ -f "$HOST_XR_JSON" ]; then
  LP="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["runtime"]["library_path"])' "$HOST_XR_JSON" 2>/dev/null)"
  PYRC=$?
  if [ $PYRC -ne 0 ]; then
    fail "cannot parse $HOST_XR_JSON (broken python3 or malformed JSON)" "check 'python3 -V' works (xcode-select --install), then inspect the file"
  elif [ "$LP" = "$OXR_DYLIB" ] && [ -f "$LP" ]; then ok "host OpenXR registration -> $LP"
  elif [ -n "$LP" ] && [ -f "$LP" ]; then warn "host registration points at $LP (expected $OXR_DYLIB)"
  else fail "host registration points at a missing dylib" "./demo.sh install --bottle $WINEVR_BOTTLE (sudo rewrites $HOST_XR_JSON)"; fi
else fail "$HOST_XR_JSON missing" "./demo.sh install --bottle $WINEVR_BOTTLE (sudo writes it)"; fi

# 13. runtime config
if [ -f "$TOML" ]; then
  PROTO="$(awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{print $2; exit}' "$TOML")"
  if [ "$PROTO" = "alvr" ]; then ok "oxrsys-runtime.toml: protocol=alvr"
  else fail "oxrsys-runtime.toml protocol='"$PROTO"' — the demo streams via ALVR" "set protocol = \"alvr\" in $TOML"; fi
else fail "$TOML missing" "./demo.sh setup"; fi
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
  if [ $PYRC -ne 0 ]; then warn "could not inspect $SESSJSON (broken python3?)"
  elif [ -n "$PINNED" ]; then
    warn "session.json pins client IP(s): $PINNED— fine while the Quest keeps that IP; if streaming stops after a DHCP change, delete '$SESSJSON' (recreated with discovery+auto-trust)"
  else ok "ALVR session state has no stale manual-IP pins"
  fi
fi

# 14. headset-side (warnings only; WiFi streaming needs no USB)
if [ -n "$ADB" ] && "$ADB" devices 2>/dev/null | awk 'NR>1 && $2=="device"' | grep -q .; then
  SER="$("$ADB" devices | awk 'NR>1 && $2=="device"{print $1; exit}')"
  ok "Quest connected via adb ($SER)"
  if "$ADB" -s "$SER" shell pm list packages 2>/dev/null | grep -q alvr; then ok "ALVR client installed on the Quest"
  else warn "ALVR client not detected on the Quest — install ALVR v20.14.1 client APK"; fi
else warn "no Quest over adb (fine for WiFi streaming; connect USB once to install the client)"; fi

# 15. audio loopback (optional)
if command -v SwitchAudioSource >/dev/null 2>&1; then
  if SwitchAudioSource -a -t output 2>/dev/null | grep -qx "BlackHole 2ch"; then ok "BlackHole 2ch + switchaudio-osx"
  else warn "BlackHole 2ch not present — no in-headset audio (brew install blackhole-2ch, then reboot)"; fi
else warn "switchaudio-osx not installed — audio stays on the Mac (brew install switchaudio-osx blackhole-2ch)"; fi

# 16. stale streaming listeners
STALE="$(lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk 'NR>1{print $1"("$2")"}' | sort -u | tr '\n' ' ')"
if [ -n "$STALE" ]; then warn "ports 9943/9944 busy: $STALE— a previous session may still be running"
else ok "streaming ports free"; fi

print ""
if [ "$FAILCOUNT" -eq 0 ]; then print -r -- "doctor: ${_G}all checks passed${_N} — ./demo.sh run --bottle $WINEVR_BOTTLE"
else print -r -- "doctor: ${_R}$FAILCOUNT check(s) failed${_N} — remedies above"; fi
[ -n "${WINEVR_DOCTOR_SOFT:-}" ] || exit "$FAILCOUNT"
