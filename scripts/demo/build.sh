# demo.sh build — build oxrsys (x86_64 + embedded ALVR core) and wineopenxr.
# Idempotent (both build systems are incremental). Sourced by demo.sh after lib.sh.
set -e

print "== wine-vr demo build =="

for tool in cmake ninja x86_64-w64-mingw32-gcc; do
  command -v $tool >/dev/null 2>&1 || die "$tool missing — brew install cmake ninja mingw-w64"
done
rustup target list --installed 2>/dev/null | grep -q x86_64-apple-darwin || \
  die "rustup x86_64-apple-darwin target missing — install rustup via https://rustup.rs and source ~/.cargo/env, then: rustup toolchain install stable && rustup target add x86_64-apple-darwin"
[ -d "$OXRSYS/runtime" ] || die "submodules not initialized — ./demo.sh setup"

# oxrsys: x86_64 (the game runs under Rosetta in wine; the runtime loads in-process),
# Debug is the live-verified configuration, ALVR core is cargo-built by cmake.
info "building oxrsys (build-x64: Ninja, Debug, x86_64, ALVR on)..."
cmake -S "$OXRSYS" -B "$OXR_BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=x86_64 -DOXRSYS_ENABLE_ALVR=ON \
  >/dev/null
cmake --build "$OXR_BUILD" -j8
ok "oxrsys built"

info "building wineopenxr (PE dll via mingw + unix .so)..."
cmake -S "$WOXR" -B "$WOXR/build" >/dev/null
cmake --build "$WOXR/build" -j8
ok "wineopenxr built"

for f in "$OXR_DYLIB" "$OXR_ALVR_DYLIB" "$OXR_RUNTIME_JSON" "$WOXR_DLL" "$WOXR_SO"; do
  [ -f "$f" ] || die "expected build output missing: $f"
done
ok "all build outputs present"
print "\nbuild complete — next: ./demo.sh install --bottle <name>"
