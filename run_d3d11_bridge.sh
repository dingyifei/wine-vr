#!/bin/zsh
# ROLE: DIAGNOSTIC — minimal D3D11 test app through the bridge (isolate the bridge without a full game).
# Launch the D3D11 OpenXR test app in the OpenXRTest bottle through the wineopenxr->oxrsys bridge.
#   d3d11_clear.exe -> libopenxr_loader.dll -> wineopenxr.dll -> wineopenxr.so (embedded loader)
#   -> XR_RUNTIME_JSON=oxrsys-x64 -> oxrsys streams to Quest (or "waiting for headset" if none).
# Usage: ./run_d3d11_bridge.sh [frames]
set -u
ROOT=~/projects/personal/wine-vr
CX=~/Applications/CrossOver.app/Contents/SharedSupport/CrossOver
BOTTLE=OpenXRTest
FRAMES="${1:-100000}"
export XR_RUNTIME_JSON="$ROOT/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json"
export CX_GRAPHICS_BACKEND=dxmt
export WINEDEBUG="${WINEDEBUG:-fixme-all,+openxr}"
mkdir -p "$ROOT/evidence"
echo "XR_RUNTIME_JSON=$XR_RUNTIME_JSON"
echo "launching d3d11_clear.exe in bottle $BOTTLE under DXMT..."
"$CX/bin/wine" --bottle "$BOTTLE" --no-update --cx-app 'C:\d3d11_clear.exe' "$FRAMES"
