# Troubleshooting

Symptom → cause → fix, from real incidents. Start with `./demo.sh doctor`: every
check prints a one-line remedy, and most rows below are things it already catches.
`./demo.sh run` re-checks the launch-critical subset before starting the game, so a
misconfiguration fails fast with the same remedy instead of a black window. Quick
start is in the [top-level README](../README.md).

## Launch

| Symptom | Cause | Fix |
|---|---|---|
| Game hangs at startup, window never appears | Stale wineserver and/or leftover Steam lock in the bottle | `./demo.sh run` kills and waits out the bottle's wineserver before every launch. Manual: `WINEPREFIX="$HOME/Library/Application Support/CrossOver/Bottles/<bottle>" "<CrossOver.app>/Contents/SharedSupport/CrossOver/bin/wineserver" -k` |
| Black window, or the game runs flat on the desktop with no VR | Bridge files in the bottle are stale, or a CrossOver update silently reverted the DXMT/wineopenxr overlay inside CrossOver.app | `./demo.sh install --bottle <bottle>`. `doctor` catches both cases (global-overlay and per-bottle checks); `run` refuses to launch while either is stale |
| Startup blocked by a Meta account prompt | Beat Saber version newer than 1.29.4 (the last pre-gate build) | Install 1.29.4 — when the version mismatches, `doctor` prints the exact DepotDownloader command |

## Video

| Symptom | Cause | Fix |
|---|---|---|
| Green or corrupted stream | Historical VideoToolbox low-latency + BGRA zero-chroma bug, fixed by feeding the encoder NV12 input. Only reappears if that fix is reverted | Keep the pinned `oxrsys-v20.14.1` ALVR branch (`./demo.sh setup` restores it). Background: [low-latency BGRA zero-chroma report](apple-feedback-1-lowlatency-bgra-zero-chroma.md) |

## Network / streaming

| Symptom | Cause | Fix |
|---|---|---|
| Client connects, then loops with EADDRINUSE on the headset | Stale adb reverse tunnels from the legacy USB path squatting port 9944 on the Quest | `./demo.sh run` clears all reverse tunnels at launch. Manual: `adb reverse --remove-all` |
| Same connect/EADDRINUSE loop, tunnels already clear | A previous server instance still alive on the Mac | `doctor` warns when ports 9943/9944 are busy and names the process; kill it and relaunch |
| Quest never connects | Mac and Quest on different WiFi networks or bands; discovery blocked by a host firewall/traffic filter (e.g. TripMode); or a stale manual-IP pin in `session.json` after a DHCP change | Put both on the same network/band; allow the traffic in the filter; delete `~/Library/Application Support/OXRSys/alvr/session.json` (recreated with discovery + auto-trust). `doctor` warns when `session.json` pins an IP |

## Audio

| Symptom | Cause | Fix |
|---|---|---|
| No in-headset audio | BlackHole 2ch not installed, or installed without rebooting afterward | `brew install blackhole-2ch switchaudio-osx`, then reboot. If the output switch fails, `run` warns and leaves audio on the Mac speakers |

## Config

| Symptom | Cause | Fix |
|---|---|---|
| `doctor` FAILs on `oxrsys-runtime.toml` | `protocol` is not `"alvr"` — hand-edited, or clobbered by an old tool | `./demo.sh setup` rewrites the file only when it is absent: delete `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` and re-run setup, or set `protocol = "alvr"` yourself |
