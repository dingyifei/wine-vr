# Beat Saber on Apple Silicon → Quest 3

Run the Windows build of **Beat Saber 1.29.4** on an Apple Silicon Mac under
CrossOver and stream it to a Meta Quest 3 over WiFi with the **stock ALVR
client** — full 6DoF tracking, 72 fps, **102.9 ms p50 / 130.5 ms p95
motion-to-photon on-head over WiFi** (native arm64 HEVC helper).

Measured status (2026-08-04, 3008x1664@72, 80 Mbps,
`evidence/batchA-soak-20260804-results.md`): WiFi desk-idle
106.9 ms p50 / 130.7 p95 over 231k frames; 60-minute soak with zero
uninduced encoder-helper deaths; ~5-6 dropped frames/s at the 80 Mbps cap
(bitrate tuning open). USB-wired measured 93.5 ms p50 — a
transport-isolation figure from the helper validation gate, not the WiFi
product number.

```
Beat Saber (Windows, x64) ─ CrossOver/Wine ─ DXMT (D3D11→Metal, zero-copy)
        └─ wineopenxr ─→ oxrsys OpenXR runtime (x86_64, compose in-process)
              └─ IOSurfaces via Mach send rights (once per generation)
                    └─ arm64 helper: VideoToolbox HW HEVC encode
                          └─ Annex-B over socketpair ─→ embedded ALVR core (x86_64) ─ WiFi/USB ─→ Quest 3
```

Everything runs in-process on the Mac; no SteamVR, no real Steam at runtime.
See [docs/architecture.md](docs/architecture.md) for how the pieces fit.

## Requirements

- **Apple Silicon Mac** (verified: M3 Max, macOS 26.x) with ~15 GB free disk
- **CrossOver 26.2+** installed in `~/Applications` or `/Applications`, with a
  **win11_64 bottle** — its name is required by every command below (examples
  use `Steam`; create one in the CrossOver UI first)
- A **Steam account that owns Beat Saber** (game files only; Steam never runs)
- **Meta Quest 3** with the **ALVR client v20.14.1** on the same 5/6 GHz WiFi
  as the Mac (install in step 4 below)
- Toolchain — requires [Homebrew](https://brew.sh); git/python3/clang come
  with the Xcode Command Line Tools (`xcode-select --install`):

  ```sh
  brew install cmake ninja mingw-w64 android-platform-tools
  brew install switchaudio-osx blackhole-2ch   # optional: in-headset audio (reboot after)
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # rustup (brew's is keg-only/not on PATH)
  source "$HOME/.cargo/env"                                        # put rustup on PATH in THIS shell
  rustup toolchain install stable && rustup target add x86_64-apple-darwin
  ```

## One-time setup

**1. Clone (submodules are fetched by setup):**

```sh
git clone https://github.com/dingyifei/wine-vr.git && cd wine-vr
```

**2. Get Beat Saber 1.29.4** — the last build before the Meta account gate and
the first with native OpenXR. Download the pinned depot with
[DepotDownloader](https://github.com/SteamRE/DepotDownloader) and your Steam
login:

```sh
DepotDownloader -app 620980 -depot 620981 -manifest 6291266771922375922 \
  -username <steam-user> -dir "<beat-saber-dir>"
```

(DepotDownloader has no brew formula: grab the `macos-arm64` release zip,
`chmod +x DepotDownloader && xattr -d com.apple.quarantine DepotDownloader`,
run as `./DepotDownloader`.)

Any `<beat-saber-dir>` works — pass it as `--bs-dir` below (default:
`<bottle>/drive_c/Program Files (x86)/Steam/steamapps/common/Beat Saber 1294`;
a directory outside the bottle is reached through the bottle's `z:` drive,
which CrossOver creates by default — `doctor` checks it).
Alternative: Steam console (`steam://open/console`) →
`download_depot 620980 620981 6291266771922375922`, then move the output there.

**3. Fetch, build, install:**

```sh
./demo.sh setup                      # submodules + sha256-pinned binaries + config
./demo.sh build                      # oxrsys (x86_64 + ALVR core) and wineopenxr
./demo.sh install --bottle Steam     # bridge into CrossOver + bottle (one sudo prompt)
```

`setup` writes the runtime config to
**`~/Library/Application Support/OXRSys/oxrsys-runtime.toml`** with
`protocol = "alvr"` and `bitrate_mbps = 42` (it never overwrites an existing
file). The embedded ALVR core keeps its `session.json` under
**`~/Library/Application Support/OXRSys/alvr/`** — note this is *not* stock
ALVR's config directory; it is auto-created on first run and LAN clients are
auto-trusted, so no pairing dance is needed.

**4. Quest client:** install **ALVR v20.14.1** on the headset — grab
`alvr_client_android.apk` from the
[ALVR v20.14.1 release](https://github.com/alvr-org/ALVR/releases/tag/v20.14.1)
and `adb install` it (or use SideQuest). Sideloading needs Developer Mode:
enable it in the Meta Horizon phone app (free developer account required) and
accept the USB-debugging prompt in the headset. The client version must match
the embedded server core; a newer store/app-lab client may refuse to pair.

## Run it

```sh
./demo.sh run --bottle Steam [--bs-dir "<beat-saber-dir>"]
```

Put the headset on and open the ALVR client; the first frame can take ~30 s.
**Pause = X/A button or the Quest system button.** (The left menu button not
pausing is a Beat Saber/Unity limitation on *every* OpenXR runtime since the
1.29.4 OpenXR port — see [docs/history/menu-button.md](docs/history/menu-button.md).)

`run` is the repeatable stage: it resets the bottle's wineserver (stale
servers hang startup), preflights everything with actionable errors, applies
the Goldberg Steam emulator to the game, routes audio into BlackHole, and
launches through the bridge. Logs land in `logs/`.

To exit: quit from the game menu, or **Ctrl-C** (tears down wine and restores
audio), or `./demo.sh stop --bottle Steam` from another shell. Note that audio
routing sends *all* Mac output to the headset while the game runs — pass
`--no-audio` to keep sound on the Mac. The console stays quiet by default;
`--verbose` enables the wine/openxr debug channels.

## Checking your setup

```sh
./demo.sh doctor --bottle Steam
```

~30 ordered checks, each failure with a one-line remedy — including the cases
that silently break later: a CrossOver update reverting the DXMT overlay, a
stale bottle, or a leftover client IP pin in `session.json`.

## Configuration

| Knob | Where | Meaning |
|---|---|---|
| `--bottle` / `WINEVR_BOTTLE` | CLI/env | CrossOver bottle name (required) |
| `--bs-dir` / `WINEVR_BS_DIR` | CLI/env | Beat Saber 1.29.4 install dir |
| `--no-audio` / `WINEVR_NO_AUDIO` | CLI/env | keep sound on the Mac (skip BlackHole routing) |
| `--no-dashboard` / `WINEVR_NO_DASHBOARD` | CLI/env | don't open the ALVR server dashboard with `run` |
| `--verbose` / `WINEVR_VERBOSE` | CLI/env | wine/openxr debug channels in console + log |
| `--wired` / `WINEVR_WIRED` | CLI/env | create the `adb forward tcp:9943/9944` pair for USB-wired streaming (a normal run clears them) |
| `protocol = "alvr"` | `oxrsys-runtime.toml` | streaming backend (demo path) |
| `bitrate_mbps` | `oxrsys-runtime.toml` | base video bitrate (template writes 42; sessions to date ran at 80, which sheds ~5-6 frames/s at the cap — tuning open; ALVR's adaptive loop adjusts from the base) |
| `encoder_process = "auto"` | `oxrsys-runtime.toml` | `auto`/`native` = out-of-process arm64 helper (HW HEVC), hard-required at launch — `run`'s preflight restages a missing/wrong-arch helper from the build tree, or fails with a remedy; `inproc` = in-process Rosetta H.264 fallback |
| `video_codec = "auto"` | `oxrsys-runtime.toml` | `h265`/`h264`/`auto` — honored for real on the helper path; the `inproc` fallback under Rosetta is H.264-only |
| `resolution_scale`, `refresh_rate_hz` | `oxrsys-runtime.toml` | encode-side downscale (1.0 = native) and refresh rate (72 verified) |

The runtime's toml parser strips same-line `#` comments (outside quotes).
Runtime builds before the 2026-08 fix silently mis-parsed keys carrying a
trailing comment — if you run an older dylib, keep comments on their own lines.

## Known limitations

- **H.264 only under the in-process fallback (`encoder_process = "inproc"`)**
  — the runtime encodes under Rosetta, where VideoToolbox HEVC paths misbehave
  (one bug is documented and filed: see
  `docs/apple-feedback-1-lowlatency-bgra-zero-chroma.md`). The default
  out-of-process arm64 helper (`auto`/`native`) encodes native HW HEVC and
  negotiates codec choice with the client for real.
- Left menu button cannot pause (game limitation, all OpenXR runtimes)
- Verified config is a Debug x86_64 build; other configs are untested

## Repo map

| Path | What |
|---|---|
| `demo.sh`, `scripts/demo/` | the demo pipeline (this page) |
| `ext/oxrsys` | submodule: OpenXR runtime + embedded ALVR backend ([fork](https://github.com/dingyifei/oxrsys)) |
| `ext/wineopenxr` | submodule: Wine OpenXR bridge ([fork](https://github.com/dingyifei/wineopenxr)) |
| `ext/ALVR` | submodule: ALVR v20.14.1 + reliability patches ([branch](https://github.com/dingyifei/ALVR/tree/oxrsys-v20.14.1)) |
| `patches/` | the ALVR patch set as a reviewable diff ([patches/README.md](patches/README.md)) |
| `docs/` | [architecture](docs/architecture.md) · [troubleshooting](docs/troubleshooting.md) · [bridge findings](docs/bridge-findings.md) · [history](docs/history/) |
| `tools/`, `scripts/dev/`, `src/` | investigation-era probes and reproducers |

Further reading: [docs/bridge-findings.md](docs/bridge-findings.md) (what was
built and why, gate by gate) and
[docs/history/steamvr-blocked.md](docs/history/steamvr-blocked.md) (why the
obvious SteamVR-under-Wine path is impossible today).
