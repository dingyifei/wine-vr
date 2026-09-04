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

# oxrsys: x86_64 (game runs under Rosetta, runtime loads in-process), Debug is the
# live-verified config. ALVR core is cargo-built by cmake (the rustup target check above).
info "building oxrsys (build-x64: Ninja, Debug, x86_64, ALVR on)..."
cmake -S "$OXRSYS" -B "$OXR_BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=x86_64 -DOXRSYS_ENABLE_ALVR=ON \
  -DOXRSYS_BUILD_ENCODER_HELPER=OFF \
  >/dev/null
cmake --build "$OXR_BUILD" -j8
ok "oxrsys built"

# Encoder helper: must be native arm64 (HW HEVC is Rosetta-blocked), so the x86_64
# tree above cannot build it — a dedicated minimal arm64 tree does (no ALVR core;
# keeps configure/build fast). The runtime finds the helper NEXT TO its own dylib,
# so stage it into build-x64/runtime/.
info "building oxrsys encoder helper (build-helper-arm64: Ninja, Debug, arm64)..."
cmake -S "$OXRSYS" -B "$OXR_HELPER_BUILD" -G Ninja \
  -DCMAKE_BUILD_TYPE=Debug -DCMAKE_OSX_ARCHITECTURES=arm64 -DOXRSYS_BUILD_ENCODER_HELPER=ON \
  >/dev/null
cmake --build "$OXR_HELPER_BUILD" --target oxrsys_encoder_helper -j8
[ -f "$OXR_HELPER_BIN_BUILT" ] || die "encoder helper build produced no binary at $OXR_HELPER_BIN_BUILT"
helper_is_arm64 "$OXR_HELPER_BIN_BUILT" || \
  die "encoder helper is not an arm64 executable ($(lipo -archs "$OXR_HELPER_BIN_BUILT" 2>/dev/null)) — delete $OXR_HELPER_BUILD and re-run ./demo.sh build"
install_if_changed "$OXR_HELPER_BIN_BUILT" "$OXR_HELPER_BIN"
ok "encoder helper built (arm64) and staged next to the runtime dylib"

info "building wineopenxr (PE dll via mingw + unix .so)..."
cmake -S "$WOXR" -B "$WOXR/build" >/dev/null
cmake --build "$WOXR/build" -j8
ok "wineopenxr built"

# Native arch (talks to the embedded server over localhost, launched by `run`).
info "building ALVR server dashboard (release)..."
(cd "$ALVR" && cargo build -p alvr_dashboard --release) || \
  die "alvr_dashboard build failed — retry with: (cd ext/ALVR && cargo build -p alvr_dashboard --release)"
ok "ALVR dashboard built"

for f in "$OXR_DYLIB" "$OXR_ALVR_DYLIB" "$OXR_RUNTIME_JSON" "$OXR_HELPER_BIN" "$WOXR_DLL" "$WOXR_SO" "$ALVR_DASHBOARD_BIN"; do
  [ -f "$f" ] || die "expected build output missing: $f"
done
ok "all build outputs present"
print "\nbuild complete — next: ./demo.sh install --bottle <name>"
