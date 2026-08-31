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
| Game boots but spins forever before any window; log stops after `Metal graphics requirements provided` with no DXMT banner; ALVR dashboard says "SteamVR is not running" | The bottle's Graphics Backend was set to "auto" (the CrossOver GUI writes `"CX_GRAPHICS_BACKEND" = ""`), which no longer selects DXMT. The bottle setting overrides `run`'s env var, so DXMT's `d3d11.dll` never loads and the game never reaches session creation — the streamer (what the dashboard calls "SteamVR") never starts | `./demo.sh run` now forces the bottle back to `dxmt` on every launch; `doctor` FAILs on any other value. Manual: CrossOver → bottle settings → Graphics Backend → DXMT |

## Video

| Symptom | Cause | Fix |
|---|---|---|
| Green or corrupted stream | Only the in-process x86_64 Rosetta fallback (`encoder_process = "inproc"`, or `auto` when the native-arm64 helper isn't staged) is affected: it feeds BGRA directly to VT (the `rgb_to_nv12` pre-convert was removed after Apple fixed VT's internal conversion in macOS 27) and older VT under Rosetta produces all-zero chroma. The native helper (`encoder_process = "native"`, HW HEVC) doesn't go through this path and is unaffected | Upgrade the host to macOS 27+, or pin oxrsys back to the NV12-era revision (`cf5f926` or earlier) — only needed if you must run the in-process fallback. `doctor` still hard-FAILs on macOS < 27 regardless of encoder path, so the fallback stays viable; verify with `tools/vt-llrc-probe --matrix`. Background: [low-latency BGRA zero-chroma report](apple-feedback-1-lowlatency-bgra-zero-chroma.md) |

## Encoder

| Symptom | Cause | Fix |
|---|---|---|
| Stream works but the log says `encoder ready … (H.264, in-process)` instead of `(HEVC, native helper)` | The staged arm64 helper is missing or wrong-arch at `ext/oxrsys/build-x64/runtime/oxrsys-encoder-helper` — under `encoder_process = "auto"` the runtime falls back to in-process H.264 (higher latency, softer image) and warns once per connection. Historically an out-of-band CMake re-configure of `build-x64` deleted the staged helper; that sweep now spares arm64 binaries | `./demo.sh run`'s preflight now restages the helper automatically from `build-helper-arm64` (or dies with a remedy); `doctor` 9b checks presence + arch. Manual: `./demo.sh build` |

## Network / streaming

| Symptom | Cause | Fix |
|---|---|---|
| Client connects, then loops with EADDRINUSE on the headset | Stale adb reverse tunnels from the legacy USB path squatting port 9944 on the Quest | `./demo.sh run` clears all reverse tunnels at launch. Manual: `adb reverse --remove-all` |
| Same connect/EADDRINUSE loop, tunnels already clear | A previous server instance still alive on the Mac | `doctor` warns when ports 9943/9944 are busy and names the process; kill it and relaunch |
| Quest never connects | Mac and Quest on different WiFi networks or bands; discovery blocked by a host firewall/traffic filter (e.g. TripMode); or a stale manual-IP pin in `session.json` after a DHCP change | Put both on the same network/band; allow the traffic in the filter; edit the pinned IP in `~/Library/Application Support/OXRSys/alvr/session.json` in place — do **not** delete the file: a recreated `session.json` streams a black 800×900 screen (proven 2026-08-10). `doctor` warns when `session.json` pins an IP |
| Quest stuck on "searching for streamer" after a wired/USB session | Stale `adb forward tcp:9943`/`tcp:9944` from a previous `--wired` launch squatting the streaming ports — forwards persist across sessions until explicitly removed | A normal (non-wired) `./demo.sh run` clears exactly these two forwards in preflight; `doctor` warns when they're present. Manual: `adb forward --remove tcp:9943` + `adb forward --remove tcp:9944` (avoid `--remove-all`, which also deletes unrelated forwards). (Unrelated to `adb reverse`, which `run` already clears every launch.) |

## Session

| Symptom | Cause | Fix |
|---|---|---|
| Headset shows a frozen frame after the display went to standby; tracking/battery still update, no disconnect | Quest display standby without an ALVR disconnect starves the frame loop: the session stays FOCUSED, encode stops, and no reconnect path fires because the client never disconnects (observed 2026-08-04, ~3 min frozen) | Fully wake the headset (press the power button / put it on) — the stream resumes by itself. No server-side fix yet; client-side behavior is under investigation |
| ALVR dashboard window is frozen (busy-spins ~34% CPU), but streaming keeps working | Upstream `alvr_dashboard` egui wedge; the server API on `127.0.0.1:8082` stays healthy | Quit and relaunch the dashboard (or ignore it — the stream doesn't depend on it; `--no-dashboard` skips it entirely) |

## Audio

| Symptom | Cause | Fix |
|---|---|---|
| No in-headset audio | BlackHole 2ch not installed, or installed without rebooting afterward | `brew install blackhole-2ch switchaudio-osx`, then reboot. If the output switch fails, `run` warns and leaves audio on the Mac speakers |
| In-headset audio quieter than expected | macOS applies the BlackHole device volume to the loopback samples before ALVR captures them | `run` now sets the BlackHole volume to 100% after switching; for a session already running, `osascript -e 'set volume output volume 100'` while BlackHole is the default output |

## Config

| Symptom | Cause | Fix |
|---|---|---|
| `doctor` FAILs on `oxrsys-runtime.toml` | `protocol` is not `"alvr"` — hand-edited, or clobbered by an old tool | `./demo.sh setup` rewrites the file only when it is absent: delete `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` and re-run setup, or set `protocol = "alvr"` yourself |
| A toml key is set but the runtime behaves as if it weren't (e.g. `video_codec = "h264" # note` still streams HEVC); the shell checks (`doctor`) pass | Runtime builds before the 2026-08 parser fix ignored same-line `#` comments: string keys silently kept code defaults and bool keys were forced to `false`, while the demo scripts' quote-splitting `awk` checks read the file correctly — so `doctor` saw the intended value and the runtime didn't | Update oxrsys (the parser now strips unquoted `#` comments), or keep comments on their own lines. The startup config-dump log line now includes `codec=` so the parsed value is visible |
| Running oxrsys test binaries directly clobbered your real config | Direct test-binary runs bypass ctest's isolated `HOME` and overwrite `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` | Always run tests through `ctest`; restore values from the startup config-dump line in an old `oxrsys-runtime.log` (see `ext/oxrsys/docs/testing-and-conformance.md`) |
