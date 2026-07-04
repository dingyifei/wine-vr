#!/bin/zsh
# SUPERSEDED by ../../demo.sh install — kept as the investigation-era reference (hardcoded OpenXRTest bottle).
# ROLE: SETUP — one-time install of the wineopenxr + DXMT-fork bridge into a CrossOver bottle.
# Install the wineopenxr + DXMT-fork bridge into the OpenXRTest CrossOver bottle.
# - Overlays monofunc DXMT fork (interop) over CrossOver's bundled DXMT (stock backed up).
# - Installs wineopenxr (PE dll + unix .so) and registers it as the bottle's OpenXR runtime.
# - Stages the D3D11 test app + the cross-built Windows OpenXR loader.
set -eu
ROOT=~/projects/personal/wine-vr
CX=~/Applications/CrossOver.app/Contents/SharedSupport/CrossOver
BOTTLE=OpenXRTest
PREFIX=~/Library/Application\ Support/CrossOver/Bottles/$BOTTLE
SYS32="$PREFIX/drive_c/windows/system32"
WINE="$CX/bin/wine"
DXMT_ART="$ROOT/ext/dxmt-artifacts"
WOXR="$ROOT/ext/wineopenxr/build"
LOADER="$ROOT/ext/wineopenxr/build-loader-win/src/loader/libopenxr_loader.dll"

echo "=== 1. Overlay DXMT fork over CrossOver bundled DXMT (backup stock once) ==="
BK="$CX/lib/dxmt.stock-backup"
if [ ! -d "$BK" ]; then cp -R "$CX/lib/dxmt" "$BK"; echo "  backed up stock DXMT -> $BK"; else echo "  stock backup already exists"; fi
for f in d3d10core.dll d3d11.dll dxgi.dll winemetal.dll; do
  cp "$DXMT_ART/x86_64-windows/$f" "$CX/lib/dxmt/x86_64-windows/$f" && echo "  fork -> dxmt/x86_64-windows/$f"
done
cp "$DXMT_ART/x86_64-unix/winemetal.so" "$CX/lib/dxmt/x86_64-unix/winemetal.so" && echo "  fork -> dxmt/x86_64-unix/winemetal.so"

echo "=== 2. Install wineopenxr ==="
cp "$WOXR/src/pe/wineopenxr.dll" "$CX/lib/wine/x86_64-windows/wineopenxr.dll" && echo "  -> CX wine x86_64-windows/wineopenxr.dll"
cp "$WOXR/src/unix/wineopenxr.so" "$CX/lib/wine/x86_64-unix/wineopenxr.so" && echo "  -> CX wine x86_64-unix/wineopenxr.so"
cp "$WOXR/src/pe/wineopenxr.dll" "$SYS32/wineopenxr.dll" && echo "  -> bottle system32/wineopenxr.dll"
mkdir -p "$PREFIX/drive_c/openxr"
cp "$ROOT/ext/wineopenxr/manifests/wineopenxr64.json" "$PREFIX/drive_c/openxr/" && echo "  -> bottle C:/openxr/wineopenxr64.json"

echo "=== 3. Register wineopenxr as the active OpenXR runtime ==="
WINEPREFIX="$PREFIX" CX_BOTTLE="$BOTTLE" "$WINE" --bottle "$BOTTLE" --no-update reg add \
  'HKLM\Software\Khronos\OpenXR\1' /v ActiveRuntime /t REG_SZ \
  /d 'C:\openxr\wineopenxr64.json' /f 2>&1 | tail -2

echo "=== 4. Stage the D3D11 test app + Windows OpenXR loader ==="
cp "$ROOT/build/d3d11_clear.exe" "$PREFIX/drive_c/d3d11_clear.exe" && echo "  -> C:/d3d11_clear.exe"
cp "$LOADER" "$SYS32/libopenxr_loader.dll" && echo "  -> system32/libopenxr_loader.dll"
# the app was linked against libopenxr_loader; also drop it next to the exe
cp "$LOADER" "$PREFIX/drive_c/libopenxr_loader.dll" && echo "  -> C:/libopenxr_loader.dll"

echo "=== done. Bottle $BOTTLE ready. ==="
