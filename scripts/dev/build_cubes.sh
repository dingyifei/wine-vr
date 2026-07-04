#!/bin/zsh
# ROLE: BUILD TOOL — builds the native Gate 1 Metal OpenXR cubes client.
# Build the native macOS Metal OpenXR cubes client (Gate 1), linking the oxrsys-fetched OpenXR loader.
set -eu
cd "$(dirname "$0")"
OX=ext/oxrsys
OXR_INC="$OX/build/_deps/openxr-src/include"
OXR_LOADER_DIR="$OX/build/_deps/openxr-build/src/loader"
mkdir -p build
clang++ -ObjC++ -std=c++17 -g -O1 -arch arm64 \
  -I"$OXR_INC" \
  src/oxrsys_cubes.mm \
  -L"$OXR_LOADER_DIR" -lopenxr_loader -Wl,-rpath,"$(cd $OXR_LOADER_DIR && pwd)" \
  -framework Foundation -framework Metal -framework QuartzCore \
  -o build/oxrsys_cubes
echo "built build/oxrsys_cubes"
file build/oxrsys_cubes
