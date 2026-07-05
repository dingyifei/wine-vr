#!/bin/zsh
# wine-vr demo dispatcher — Beat Saber 1.29.4 on Apple Silicon -> Quest 3.
#
#   ./demo.sh doctor  --bottle <name>   # check every prerequisite, print remedies
#   ./demo.sh setup                     # fetch submodules + pinned binaries, write config
#   ./demo.sh build                     # build oxrsys (with ALVR core) + wineopenxr
#   ./demo.sh install --bottle <name>   # install the bridge (one sudo prompt)
#   ./demo.sh run     --bottle <name>   # launch Beat Saber through the bridge
#   ./demo.sh stop    --bottle <name>   # cleanly stop the game + wineserver
#   ./demo.sh all     --bottle <name>   # setup + build + install + run
#
# Ctrl-C during `run` also exits cleanly (tears down wine, restores audio).
#
# Options (doctor/install/run/stop/all):
#   --bottle <name>   CrossOver bottle (required; or env WINEVR_BOTTLE)
#   --bs-dir <path>   Beat Saber 1.29.4 install dir (or env WINEVR_BS_DIR);
#                     default: <bottle>/drive_c/Program Files (x86)/Steam/
#                              steamapps/common/Beat Saber 1294
#   --no-audio        don't route Mac audio into the headset (run only)
#   --verbose         full wine debug channels in the console/log (run only)
set -u

ROOT="$(cd "$(dirname "$0")" && pwd)"
export WINEVR_ROOT="$ROOT"

CMD="${1:-}"
[ $# -gt 0 ] && shift
while [ $# -gt 0 ]; do
  case "$1" in
    --bottle) [ $# -ge 2 ] || { echo "error: --bottle needs a name" >&2; exit 2; }
              export WINEVR_BOTTLE="$2"; shift 2 ;;
    --bs-dir) [ $# -ge 2 ] || { echo "error: --bs-dir needs a path" >&2; exit 2; }
              export WINEVR_BS_DIR="$2"; shift 2 ;;
    --no-audio) export WINEVR_NO_AUDIO=1; shift ;;
    --verbose)  export WINEVR_VERBOSE=1; shift ;;
    *) echo "error: unknown argument '$1'" >&2; exit 2 ;;
  esac
done

source "$ROOT/scripts/demo/lib.sh"

case "$CMD" in
  doctor|setup|build|install|run|stop)
    source "$ROOT/scripts/demo/$CMD.sh" ;;
  all)
    require_bottle   # fail fast before the expensive fetch/build stages
    for stage in setup build install run; do
      echo "\n##### demo.sh: $stage #####"
      "$ROOT/demo.sh" "$stage" || exit $?   # WINEVR_* travel via the environment
    done ;;
  *)
    sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
    exit 2 ;;
esac
