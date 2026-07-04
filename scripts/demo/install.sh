# demo.sh install — install the bridge into CrossOver + the bottle + the host loader.
# Idempotent (hash-gated copies). The ONLY stage that needs sudo (host loader JSON).
# Sourced by demo.sh after lib.sh.
set -e

print "== wine-vr demo install =="
require_bottle
[ -n "${CX_APP:-}" ] || die "CrossOver.app not found"
for f in "$OXR_DYLIB" "$WOXR_DLL" "$WOXR_SO"; do
  [ -f "$f" ] || die "missing build output $f — ./demo.sh build first"
done
dxmt_files_ok || die "ext/dxmt-artifacts missing or incomplete — ./demo.sh setup first (never half-applies the overlay)"

# 1. global DXMT overlay (fork adds cross-process texture interop; stock backed up once)
print -r -- "-- global DXMT overlay ($CX/lib/dxmt)"
BK="$CX/lib/dxmt.stock-backup"
if [ ! -d "$BK" ]; then cp -R "$CX/lib/dxmt" "$BK"; ok "backed up stock DXMT -> $BK"
else info "stock DXMT backup already exists"; fi
for f in d3d10core.dll d3d11.dll dxgi.dll winemetal.dll; do
  install_if_changed "$DXMT_ART/x86_64-windows/$f" "$CX/lib/dxmt/x86_64-windows/$f"
done
install_if_changed "$DXMT_ART/x86_64-unix/winemetal.so" "$CX/lib/dxmt/x86_64-unix/winemetal.so"

# 2. global wineopenxr
print -r -- "-- global wineopenxr ($CX/lib/wine)"
install_if_changed "$WOXR_DLL" "$CX/lib/wine/x86_64-windows/wineopenxr.dll"
install_if_changed "$WOXR_SO"  "$CX/lib/wine/x86_64-unix/wineopenxr.so"

# 3. per-bottle: dll + OpenXR manifest + ActiveRuntime registry key
print -r -- "-- bottle '$WINEVR_BOTTLE'"
install_if_changed "$WOXR_DLL" "$SYS32/wineopenxr.dll"
mkdir -p "$PREFIX/drive_c/openxr"
install_if_changed "$WOXR/manifests/wineopenxr64.json" "$PREFIX/drive_c/openxr/wineopenxr64.json"
if grep -q 'ActiveRuntime.*openxr.*wineopenxr64.json' "$PREFIX/system.reg" 2>/dev/null; then
  info "registry ActiveRuntime already set"
else
  info "registering wineopenxr as the bottle's OpenXR runtime (starts wine briefly)..."
  WINEPREFIX="$PREFIX" CX_BOTTLE="$WINEVR_BOTTLE" "$WINE" --bottle "$WINEVR_BOTTLE" --no-update reg add \
    'HKLM\Software\Khronos\OpenXR\1' /v ActiveRuntime /t REG_SZ \
    /d 'C:\openxr\wineopenxr64.json' /f >/dev/null 2>&1 || die "reg add failed"
  grep -q 'ActiveRuntime' "$PREFIX/system.reg" || warn "registry write not yet visible in system.reg (wine flushes lazily) — re-run doctor later"
  ok "ActiveRuntime registered"
fi

# 4. host loader registration. The macOS OpenXR loader inside wine's secure-exec
#    ignores XR_RUNTIME_JSON; this root-owned file is what actually routes the
#    game to the oxrsys runtime.
print -r -- "-- host OpenXR registration ($HOST_XR_JSON)"
WANT="{
    \"file_format_version\": \"1.0.0\",
    \"runtime\": {
        \"name\": \"OXRSys Runtime\",
        \"library_path\": \"$OXR_DYLIB\"
    }
}"
if [ -f "$HOST_XR_JSON" ] && [ "$(cat "$HOST_XR_JSON")" = "$WANT" ]; then
  info "host registration already current"
else
  info "writing $HOST_XR_JSON (needs sudo)..."
  sudo mkdir -p "$(dirname "$HOST_XR_JSON")" || die "sudo mkdir failed"
  print -- "$WANT" | sudo tee "$HOST_XR_JSON" >/dev/null || die "sudo write failed"
  ok "host registration written"
fi

print "\ninstall complete — next: ./demo.sh run --bottle $WINEVR_BOTTLE"
