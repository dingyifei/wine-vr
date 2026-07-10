# demo.sh stop — cleanly stop the game and the bottle's wine processes.
# Sourced by demo.sh after lib.sh.

print "== wine-vr demo stop =="
require_bottle

print -r -- "-- stopping wineserver for bottle '$WINEVR_BOTTLE' (takes the game with it)"
stop_wine
if pgrep -f 'Beat Saber.exe' >/dev/null 2>&1; then
  warn "Beat Saber processes survived: $(pgrep -lf 'Beat Saber.exe' | tr '\n' ' ')"
else
  ok "game and wineserver down"
fi

STALE="$(lsof -nP -iUDP:9944 -iTCP:9943 2>/dev/null | awk 'NR>1{print $1"("$2")"}' | sort -u | tr '\n' ' ')"
if [ -n "$STALE" ]; then warn "streaming ports still held by: $STALE"
else ok "streaming ports free"; fi

if pgrep -f "$ALVR_DASHBOARD_BIN" >/dev/null 2>&1; then
  pkill -f "$ALVR_DASHBOARD_BIN" 2>/dev/null || true
  ok "ALVR dashboard closed (left over from a run that died uncleanly)"
fi

if command -v SwitchAudioSource >/dev/null 2>&1; then
  CUR="$(SwitchAudioSource -c -t output 2>/dev/null)"
  if [ "$CUR" = "BlackHole 2ch" ]; then
    warn "Mac audio output is still BlackHole 2ch (a run that died uncleanly could not restore it)"
    info "restore with: SwitchAudioSource -t output -s '<device>'   (list: SwitchAudioSource -a -t output)"
  else
    ok "audio output: $CUR"
  fi
fi
