#!/bin/zsh
# Build the Stage-0 alvr_server_core smoke test (x86_64, matching the
# architecture the oxrsys runtime uses under Wine/Rosetta).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ALVR_DIR="$SCRIPT_DIR/../../ext/ALVR"
DYLIB_DIR="$ALVR_DIR/target/x86_64-apple-darwin/release/deps"
HEADER_DIR="$SCRIPT_DIR/../../ext/oxrsys/runtime/src/alvr"

clang++ -arch x86_64 -std=c++17 -O1 \
    -I "$HEADER_DIR" \
    "$SCRIPT_DIR/main.cpp" \
    -L "$DYLIB_DIR" -lalvr_server_core \
    -o "$SCRIPT_DIR/alvr-smoke"

echo "built: $SCRIPT_DIR/alvr-smoke"
echo "run:   $SCRIPT_DIR/alvr-smoke /tmp/alvr-smoke/config /tmp/alvr-smoke/logs"
