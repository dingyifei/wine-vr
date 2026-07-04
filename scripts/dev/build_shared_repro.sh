#!/bin/zsh
# ROLE: BUILD TOOL — builds the D3D11 shared-resource reproducer (investigation).
# Build the D3D11 shared-resource reproducer for Windows x64 using mingw-w64.
set -e
cd "$(dirname "$0")/../.."   # repo root (script moved to scripts/dev/)
CXX=x86_64-w64-mingw32-g++
command -v $CXX >/dev/null || { echo "mingw-w64 not found"; exit 1; }
$CXX -std=c++17 -O2 src/shared_repro.cpp \
    -o shared_repro.exe \
    -ld3d11 -ldxgi -ldxguid -luuid -static -static-libgcc -static-libstdc++
echo "built: shared_repro.exe"
file shared_repro.exe
