#!/bin/zsh
# ROLE: SUPPORTING — brings up the Quest streaming client (adb reverse + launch); used by the primary path.
# Gate 1 live test: install the oxrsys Quest client, wire USB ADB reverse ports,
# launch the client on the Quest, and run the native cubes app so oxrsys streams to the headset.
# Prereq: Quest connected via USB-C, developer mode on, USB debugging authorized.
set -u
cd "$(dirname "$0")/../.."   # repo root (script moved to scripts/dev/)
ADB=~/Library/Android/sdk/platform-tools/adb
APK=ext/oxrsys/clients/Android/android-vr/app/build/outputs/apk/debug/app-debug.apk
PKG=net.demonixis.oxrsys.android
export XR_RUNTIME_JSON="$PWD/ext/oxrsys/build/runtime/oxrsys-runtime.json"

SER=$("$ADB" devices | awk 'NR>1 && $2=="device"{print $1; exit}')
[ -z "$SER" ] && { echo "No authorized Quest in 'adb devices'. Connect via USB-C, put on headset, Allow USB debugging."; "$ADB" devices -l; exit 1; }
echo "Quest serial: $SER"

echo "=== install APK ==="; "$ADB" -s "$SER" install -r "$APK" 2>&1 | tail -2
echo "=== clear stale reverse maps + set ports ==="
"$ADB" -s "$SER" reverse --remove-all 2>/dev/null
for p in 9944 9945 9946 9948; do "$ADB" -s "$SER" reverse tcp:$p tcp:$p && echo "  reverse $p ok"; done
echo "=== launch Quest client ==="
"$ADB" -s "$SER" shell am start -n "$PKG/com.oculus.NativeActivity" 2>&1 | tail -2
echo "=== (optional) headset logcat -> evidence/gate1-quest-logcat.txt ==="
"$ADB" -s "$SER" logcat -c 2>/dev/null
"$ADB" -s "$SER" logcat -v time -s 'OXRSys-Android:*' 'OXRSys-Network:*' 'OXRSys-Decoder:*' > evidence/gate1-quest-logcat.txt 2>&1 &
LOGPID=$!
echo "=== run native cubes app (Ctrl-C to stop) ==="
./build/oxrsys_cubes 100000 2>&1 | tee evidence/gate1-cubes-streaming.txt
kill $LOGPID 2>/dev/null
