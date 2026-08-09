# GENERATED from contract/ — DO NOT EDIT. Regenerate: scripts/dev/parity.sh --regen
# contract-sha256: 50a077e8d92effae30896e3adaf824bcdc216d7fc644bfba8b50b3c51a4329c0
# Shared scalar contract between demo.sh (this file is sourced by lib.sh) and
# sabrage-core (which parses contract/pipeline.toml directly). Values here MUST
# match contract/pipeline.toml — doctor's meta.contract-sync check verifies the
# header hash above against a live recompute of the contract/ files.

# ---- pinned dependency sources -----------------------------------------------
DEPS_URL="https://github.com/dingyifei/wine-vr/releases/download/deps-v1"
DXMT_TGZ_SHA256="487e57e86e9866c922f8d8e42a50cb0818697b927739b6741fae8f4447e2df96"
GBE_DLL_SHA256="cc5a2c9cb93fdbde7dadb825138ab7f694e3f8c310cdd675f733eaa784cbcc3e"

# ---- Beat Saber depot pin ------------------------------------------------------
BS_APPID=620980
BS_DEPOT=620981
BS_MANIFEST=6291266771922375922

# ---- host OpenXR loader registration -------------------------------------------
HOST_XR_JSON="/usr/local/share/openxr/1/active_runtime.x86_64.json"

# ---- DXMT artifact set (presence gates key on ALL of these) --------------------
DXMT_FILES=(x86_64-windows/d3d10core.dll x86_64-windows/d3d11.dll x86_64-windows/dxgi.dll x86_64-windows/winemetal.dll x86_64-unix/winemetal.so)

# ---- streaming ports -----------------------------------------------------------
WIRED_PORTS=(9943 9944)
LEGACY_REVERSE_PORTS=(9944 9945 9946 9948)   # 9947 deliberately absent
