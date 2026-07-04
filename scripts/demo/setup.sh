# demo.sh setup — one-time fetch of sources + pinned binaries and config bootstrap.
# Idempotent; no sudo. Sourced by demo.sh after lib.sh.
set -e

print "== wine-vr demo setup =="

# 1. submodules (oxrsys, wineopenxr, patched ALVR — pinned by the superproject)
info "initializing submodules (first ALVR fetch is large)..."
git -C "$ROOT" submodule update --init ext/oxrsys ext/wineopenxr ext/ALVR
git -C "$WOXR" submodule update --init          # OpenXR-SDK + wine headers (build deps)
git -C "$ALVR" submodule update --init          # openvr (alvr_session build dep)
[ -f "$ALVR/openvr/headers/openvr_driver.h" ] || \
  die "ALVR openvr submodule did not materialize — check network/auth and re-run setup"
ok "submodules ready"
if grep -q is_streaming_nonblocking "$ALVR/alvr/server_core/src/connection.rs"; then
  ok "ALVR checkout carries the oxrsys patch set (branch oxrsys-v20.14.1)"
else
  die "ALVR submodule is missing the oxrsys patches — run: git -C \"$ROOT\" submodule update --checkout ext/ALVR"
fi

# 2. pinned binaries (sha256-verified from the deps-v1 release)
if [ -f "$GBE_DLL" ] && ! sha256_ok "$GBE_DLL" "$GBE_DLL_SHA256"; then
  warn "Goldberg dll present with a non-pinned hash — keeping it (delete $GBE_DLL to re-fetch the pinned build)"
else
  fetch_pinned "$DEPS_URL/gbe-steam_api64-regular-x64.dll" "$GBE_DLL" "$GBE_DLL_SHA256" \
    "Goldberg Steam emulator dll"
fi
if dxmt_ok; then
  info "already present: dxmt-artifacts (sha256 marker matches)"
else
  fetch_pinned "$DEPS_URL/dxmt-artifacts-monofunc.tar.gz" \
    "$ROOT/third_party/downloads/dxmt-artifacts-monofunc.tar.gz" "$DXMT_TGZ_SHA256" \
    "DXMT fork artifacts"
  rm -rf "$DXMT_ART"
  tar -xzf "$ROOT/third_party/downloads/dxmt-artifacts-monofunc.tar.gz" -C "$ROOT/ext" || die "extraction failed"
  dxmt_files_ok || die "extracted dxmt-artifacts are incomplete — delete $DXMT_ART and re-run setup"
  print -r -- "$DXMT_TGZ_SHA256" > "$DXMT_ART/.sha256"
  ok "extracted ext/dxmt-artifacts (provenance marker written)"
fi

# 3. runtime config (~/Library/Application Support/OXRSys) — never clobber an existing file
mkdir -p "$OXR_APPSUP"
if [ -f "$TOML" ]; then
  PROTO="$(awk -F'"' '/^[[:space:]]*protocol[[:space:]]*=/{print $2; exit}' "$TOML")"
  if [ "$PROTO" = "alvr" ]; then info "config present: $TOML (protocol=alvr)"
  else warn "config present with protocol='"$PROTO"' — the demo needs protocol = \"alvr\"; edit $TOML yourself (not overwriting)"; fi
else
  cat > "$TOML" <<'EOF'
# oxrsys runtime configuration (created by wine-vr demo.sh setup)
[streaming]
protocol = "alvr"     # embedded ALVR core; stock ALVR Quest client connects over WiFi
bitrate_mbps = 42
EOF
  ok "wrote $TOML (protocol=alvr, 42 Mbps)"
fi
info "note: the embedded ALVR core keeps its session.json under '$OXR_APPSUP/alvr/' — auto-created on first run, LAN clients auto-trusted"

# 4. Beat Saber presence (never automated — needs your Steam account)
if [ -n "${WINEVR_BOTTLE:-}" ] || [ -n "${WINEVR_BS_DIR:-}" ]; then
  [ -n "${WINEVR_BOTTLE:-}" ] && require_bottle || {
    BS_DIR="$WINEVR_BS_DIR"
    DEPOT_CMD="DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 -username <steam-user> -dir \"$BS_DIR\""
  }
  if [ -f "$BS_DIR/Beat Saber.exe" ]; then ok "Beat Saber found at $BS_DIR"
  else
    warn "Beat Saber 1.29.4 not found at $BS_DIR"
    info "download it with your Steam account (owning Beat Saber):"
    info "  $DEPOT_CMD"
  fi
else
  info "Beat Saber check skipped (no --bottle/--bs-dir given); ./demo.sh doctor will verify it"
fi

print "\nsetup complete — next: ./demo.sh build"
