#!/bin/zsh
# ROLE: PRIMARY launcher — Beat Saber 1.29.4 end-to-end through the bridge (current working path).
# End-to-end launch of the downgraded Beat Saber 1.29.4 through the wine-vr / oxrsys bridge,
# with Goldberg Steam Emulator so NO real Steam is needed (solves the Meta gate + Steam instability).
#
# Prereq: DepotDownloader has already pulled Beat Saber 1.29.4 (app 620980, depot 620981,
#         manifest 6291266771922375922) into  "$BS_DIR"  below.
# Usage:  ./run_beatsaber_1294.sh [--goldberg regular|experimental] [frames]
set -u
ROOT=~/projects/personal/wine-vr
CX=~/Applications/CrossOver.app/Contents/SharedSupport/CrossOver
BOTTLE=Steam
BS_MAC="/Users/yifeiding/wine_shared/SteamLibrary/steamapps/common/Beat Saber 1294"
BS_WIN='Y:\SteamLibrary\steamapps\common\Beat Saber 1294\Beat Saber.exe'
GBE_FLAVOR="regular"
GBE_SRC="/tmp/gbe/release/${GBE_FLAVOR}/x64/steam_api64.dll"
ADB=~/Library/Android/sdk/platform-tools/adb
SER=$("$ADB" devices 2>/dev/null | awk 'NR>1 && $2=="device"{print $1; exit}')

# ---- 1. Verify the downgraded install ---------------------------------------
echo "===== 1. VERIFY DOWNLOAD ====="
if [ ! -f "$BS_MAC/Beat Saber.exe" ]; then
  echo "FATAL: '$BS_MAC/Beat Saber.exe' missing. Run DepotDownloader first."; exit 1
fi
VER=$(cat "$BS_MAC/BeatSaberVersion.txt" 2>/dev/null || echo "?")
echo "BeatSaberVersion.txt: $VER   (expected 1.29.4_...)"
echo "key files:"
for f in "Beat Saber.exe" "UnityPlayer.dll" "Beat Saber_Data/globalgamemanagers"; do
  if [ -e "$BS_MAC/$f" ]; then printf '  OK  %s\n' "$f"; else printf '  MISSING %s\n' "$f"; fi
done
case "$VER" in 1.29.4*) : ;; *) echo "WARN: version is not 1.29.4 — the Meta gate may still be present." ;; esac

# ---- 2. Apply Goldberg (no real Steam needed) -------------------------------
echo "\n===== 2. GOLDBERG ====="
if [ ! -f "$GBE_SRC" ]; then echo "FATAL: goldberg dll missing at $GBE_SRC"; exit 1; fi
API="$BS_MAC/Beat Saber_Data/Plugins/x86_64/steam_api64.dll"
[ -f "$API" ] || API="$BS_MAC/steam_api64.dll"          # some builds keep it beside the exe
APIDIR=$(dirname "$API")
if [ -f "$API" ] && [ ! -f "$API.orig-steam" ]; then
  cp "$API" "$API.orig-steam"; echo "backed up real steam_api64.dll -> $API.orig-steam"
fi
cp "$GBE_SRC" "$API"; echo "installed goldberg ($GBE_FLAVOR) -> $API"
printf '620980' > "$APIDIR/steam_appid.txt"; echo "wrote steam_appid.txt (620980) in $APIDIR"
# Goldberg settings: force offline, disable networking/overlay to avoid any hang.
GSET="$APIDIR/steam_settings"; mkdir -p "$GSET"
: > "$GSET/offline.txt"; : > "$GSET/disable_networking.txt"; : > "$GSET/disable_overlay.txt"
echo "goldberg steam_settings: offline + no networking + no overlay"

# ---- 3. Quest client (best-effort; needs headset worn to start immersive) ---
echo "\n===== 3. QUEST CLIENT ====="
# Streaming protocol from oxrsys config: "alvr" uses the stock ALVR client and
# ALVR's own adb management; the oxrsys reverse tunnels below would squat the
# ALVR client's stream port (9944 EADDRINUSE on the headset), so skip them.
OXR_TOML="$HOME/Library/Application Support/OXRSys/oxrsys-runtime.toml"
PROTOCOL=$(grep -E '^protocol *= *' "$OXR_TOML" 2>/dev/null | sed -E 's/.*"(.*)".*/\1/')
if [ "$PROTOCOL" = "alvr" ] && command -v SwitchAudioSource >/dev/null 2>&1; then
  # Route game audio into the BlackHole loopback so ALVR can capture it and
  # stream it to the headset. Restore the previous output on exit.
  if SwitchAudioSource -a -t output | grep -q "BlackHole"; then
    PREV_AUDIO_OUT=$(SwitchAudioSource -c -t output)
    SwitchAudioSource -t output -s "BlackHole 2ch" >/dev/null && \
      echo "audio: default output -> BlackHole 2ch (was: $PREV_AUDIO_OUT)"
    trap '[ -n "$PREV_AUDIO_OUT" ] && SwitchAudioSource -t output -s "$PREV_AUDIO_OUT" >/dev/null 2>&1 && echo "audio: restored output -> $PREV_AUDIO_OUT"' EXIT
  else
    echo "audio: BlackHole device not present yet (reboot needed after install); skipping headset audio routing"
  fi
fi
if [ -n "$SER" ] && [ "$PROTOCOL" = "alvr" ]; then
  echo "Quest: $SER (protocol=alvr — stock ALVR client, server manages adb itself)"
  "$ADB" -s "$SER" reverse --remove-all 2>/dev/null
  echo "  cleared oxrsys reverse tunnels; ALVR server_core sets up its own forwards"
elif [ -n "$SER" ]; then
  echo "Quest: $SER"
  "$ADB" -s "$SER" reverse --remove-all 2>/dev/null
  for p in 9944 9945 9946 9948; do "$ADB" -s "$SER" reverse tcp:$p tcp:$p >/dev/null && echo "  reverse $p ok"; done
  echo "launching oxrsys client (put the headset ON so Horizon OS will start the immersive activity)..."
  "$ADB" -s "$SER" shell am start -n net.demonixis.oxrsys.android/com.oculus.NativeActivity 2>&1 | tail -1
else
  echo "WARN: no Quest in 'adb devices' — connect USB-C, then re-run just the client bring-up."
fi

# ---- 4. Launch Beat Saber through the bridge (no Steam) ----------------------
echo "\n===== 4. LAUNCH BEAT SABER (bridge, no Steam) ====="
export XR_RUNTIME_JSON="$ROOT/ext/oxrsys/build-x64/runtime/oxrsys-runtime.json"
export CX_GRAPHICS_BACKEND=dxmt
export WINEDEBUG="${WINEDEBUG:-fixme-all,+openxr}"
export SteamAppId=620980 SteamGameId=620980   # help goldberg/steam_api resolve appid
echo "XR_RUNTIME_JSON=$XR_RUNTIME_JSON"
echo "CX_GRAPHICS_BACKEND=$CX_GRAPHICS_BACKEND"
echo "exe: $BS_WIN"
LOG="$ROOT/evidence/beatsaber-1294-$(date +%H%M%S).log"
mkdir -p "$ROOT/evidence"
echo "wine stderr/stdout -> $LOG"
"$CX/bin/wine" --bottle "$BOTTLE" --no-update --cx-app "$BS_WIN" 2>&1 | tee "$LOG"
