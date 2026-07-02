#!/bin/zsh
# ROLE: DIAGNOSTIC — compares CrossOver D3D11 backends' shared-resource support (investigation).
# Run shared_repro.exe under each CrossOver D3D11 backend, capturing output per backend.
# Mechanism: CX_GRAPHICS_BACKEND selects the D3D11->{Metal|Vulkan} translator.
#   values: wined3d | dxvk | dxmt | d3dmetal
set -u
cd "$(dirname "$0")"
CXAPP=~/Applications/CrossOver.app/Contents/SharedSupport/CrossOver
WINE="$CXAPP/bin/wine"
BOTTLE="Steam"
EXE_SRC="$PWD/shared_repro.exe"
PREFIX=~/Library/Application\ Support/CrossOver/Bottles/$BOTTLE
# stage exe into the bottle
cp "$EXE_SRC" "$PREFIX/drive_c/shared_repro.exe"
mkdir -p evidence

for BK in wined3d dxvk dxmt d3dmetal; do
  echo "############# BACKEND: $BK #############"
  OUT="evidence/repro-$BK.txt"
  CX_GRAPHICS_BACKEND=$BK WINEDEBUG=+loaddll \
    "$WINE" --bottle "$BOTTLE" --cx-app "C:\\shared_repro.exe" \
    >"$OUT" 2>&1
  echo "--- $BK: which d3d11 loaded ---" | tee -a "$OUT"
  grep -iE "Loaded.*(d3d11|dxgi|dxvk|dxmt|wined3d)" "$OUT" | tail -5
  echo "--- $BK: results ---"
  grep -E "hr=0x|feature level|FATAL" "$OUT"
  echo
done
