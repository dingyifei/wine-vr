#!/bin/zsh
# ROLE: DIAGNOSTIC — Gate 0 IOSurface/Rosetta cross-process probe (investigation).
# run_gate0b.sh <creator-arch> <importer-arch>
# Creator makes a global IOSurface (Metal-filled checkerboard); importer looks it up by ID,
# imports via VK_EXT_metal_objects, reads back, verifies. Tests the Rosetta-x86 <-> arm64 boundary.
set -u
cd "$(dirname "$0")/../.."   # repo root (script moved to scripts/dev/)
CA="${1:-arm64}"; IA="${2:-x86_64}"
IDF="/tmp/gate0_surfid_${CA}_${IA}.txt"
rm -f "$IDF"
echo "=== creator=$CA  importer=$IA ==="
./build/gate0_iosurf.$CA create "$IDF" > "evidence/gate0b-create-$CA.txt" 2>&1 &
NPID=$!
for i in $(seq 1 100); do [ -s "$IDF" ] && break; sleep 0.1; done
ID=$(cat "$IDF" 2>/dev/null)
echo "IOSurfaceID=$ID"
./build/gate0_iosurf.$IA import "$IDF" > "evidence/gate0b-import-${CA}-to-${IA}.txt" 2>&1
RC=$?
kill $NPID 2>/dev/null
echo "importer exit=$RC"
echo "--- importer log ---"; cat "evidence/gate0b-import-${CA}-to-${IA}.txt"
exit $RC
