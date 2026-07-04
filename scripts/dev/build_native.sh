#!/bin/zsh
# ROLE: BUILD TOOL — builds the Gate 0 native IOSurface probe.
# Build the Gate 0 native IOSurface probe.
#   ./build_native.sh arm64    -> links Homebrew MoltenVK 1.4.1 (native arm64)
#   ./build_native.sh x86_64   -> links CrossOver's MoltenVK 1.2.10 directly (Rosetta path,
#                                 the exact MoltenVK the Wine bottle uses)
# Both link MoltenVK DIRECTLY (no Vulkan loader) so the same binary needs no ICD env.
set -eu
cd "$(dirname "$0")"
ARCH="${1:-arm64}"
HDR=/opt/homebrew/Cellar/vulkan-headers/1.4.350.1/include   # headers are arch-independent
OUT="build/gate0_iosurf.$ARCH"
mkdir -p build

if [ "$ARCH" = "arm64" ]; then
  MVK_DIR=/opt/homebrew/lib
else
  MVK_DIR="$HOME/Applications/CrossOver.app/Contents/SharedSupport/CrossOver/lib64"
fi
[ -e "$MVK_DIR/libMoltenVK.dylib" ] || { echo "no libMoltenVK.dylib in $MVK_DIR"; exit 1; }

clang++ -ObjC++ -std=c++17 -g -O1 -arch "$ARCH" \
  -I"$HDR" \
  src/gate0_iosurf.mm \
  -L"$MVK_DIR" -lMoltenVK -Wl,-rpath,"$MVK_DIR" \
  -framework Foundation -framework Metal -framework IOSurface -framework CoreFoundation \
  -o "$OUT"
echo "built $OUT  (MoltenVK: $MVK_DIR)"
file "$OUT"
