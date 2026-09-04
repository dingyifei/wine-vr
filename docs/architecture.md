# Architecture

How a Beat Saber frame gets from a Windows x64 process on an Apple Silicon Mac
into a Quest 3 headset, and which repository owns each hop. For setup and
usage, see the [README](../README.md).

## Frame path

```
[x86_64 Wine process, under Rosetta]
Beat Saber.exe (x64, CrossOver/Wine, Rosetta)
  └─ openxr_loader.dll (Unity's stock loader)
      └─ wineopenxr.dll (PE) ⇄ wineopenxr.so (unix side, same process)
            │  D3D11 → Metal via DXMT; swapchain images imported as MTLTextures (zero-copy)
            └─ oxrsys runtime (in-process dylib, x86_64 under Rosetta)
                  └─ BGRA composite (3 IOSurface-backed slots)
                        └─ IOSurfaces handed off as Mach send rights (once per generation)
                              │
[oxrsys-encoder-helper, native arm64 process] ◄──────────┘
                  └─ VideoToolbox HW HEVC Main, low-latency encode
                        └─ Annex-B NALs back over an inherited socketpair
                              │
[back in the x86_64 Wine process]
                  └─ embedded alvr_server_core ── WiFi/USB ──► stock ALVR client (Quest 3)

(encoder_process = "inproc" fallback: BGRA composite → VideoToolbox H.264
 encode directly in the x86_64 process, no helper hop, macOS 27+ only)
```

The game renders D3D11; the DXMT fork translates that to Metal. Unity's own
`openxr_loader.dll` selects `wineopenxr.dll` as the active Windows OpenXR
runtime, and every call is thunked to `wineopenxr.so`, which runs as host code
inside the same process.

No pixel copies happen along the way. The oxrsys runtime allocates the
swapchain images as MTLTextures; wineopenxr hands them to the D3D11 device
through DXMT's interop interface (`IMTLD3D11InteropDevice::ImportMTLTexture2D`),
so the game draws directly into runtime-owned textures. Frame completion is a
real GPU fence: DXMT exposes the D3D11 fence's `MTLSharedEvent`
(`GetFenceSharedEvent`) and the unix side waits on it before releasing the
frame.

Runtime discovery is the one non-obvious hop: Wine's secure-exec path ignores
`XR_RUNTIME_JSON`, so the host OpenXR loader finds oxrsys only via the
root-owned system manifest `/usr/local/share/openxr/1/active_runtime.x86_64.json`
(installed by `./demo.sh install`). The whole Wine process is x86_64 under
Rosetta, so the manifest points at an x86_64 build of the runtime, loaded
in-process.

By default (`encoder_process = "auto"`, effectively `"native"`), encoding
happens out of process on a native arm64 helper (`oxrsys-encoder-helper`)
spawned by the x86_64 Wine parent. The parent keeps compose (3
IOSurface-backed BGRA slots) and every `alvr_*` call; only the IOSurfaces
cross the process boundary, once per generation, and the helper encodes
VideoToolbox HW HEVC Main with low-latency rate control natively on arm64 —
no Rosetta, no chroma bug. The spawn/rendezvous/wire mechanism lives with the
runtime: see `ext/oxrsys/docs/architecture.md`. Crash handling is budgeted:
at most one automatic respawn per 30 s, and after two failures in one
connected session `auto` pins to in-process H.264 until the next reconnect
(`native` fails loudly instead). Live recovery numbers (2026-08-04):
SIGKILL→HEVC-ready 398 ms (515 ms mid-soak), second-failure pin engaged in
13 ms, parent kill-9 → helper EOF-exit in 0.32 s.

Latency, by transport (3008x1664@72, 80 Mbps, all p50 unless noted):

| | USB-wired (Gate E, 2026-08-03) | WiFi desk-idle (2026-08-04) | WiFi on-head (2026-08-04) |
|---|---|---|---|
| total motion-to-photon | 93.5 ms | 106.9 ms (130.7 p95) | 102.9 ms (130.5 p95) |
| encoder | 37.6 ms | 38.2 ms | 38.6 ms |
| network | — | 7.4 ms | 7.0 ms |
| Quest decoder | 16.6 ms | 16.7 ms | 16.9 ms |
| decoder_queue | 1.1 ms | 1.9 ms (17.2 p95) | 2.1 ms |
| vsync_queue | 22.9 ms | 24.4 ms (46.0 p95) | 25.0 ms |

(The pre-helper in-process WiFi H.264 baseline was ~114 ms; the helper's HEVC
collapsed the Quest `decoder_queue` from ~30 ms. `vsync_queue` — frames
arriving early and waiting for display — is the frame-pacing headroom.)

The in-process Rosetta fallback (`encoder_process = "inproc"`) is retained:
on `xrEndFrame`, the runtime composites layers into a BGRA target that is
itself the encoder's CVPixelBuffer, and encodes with a VideoToolbox H.264
session using `EnableLowLatencyRateControl`; VT performs the BT.709
video-range RGB→YCbCr conversion internally (declared via session/buffer color
attachments, verified numerically by `tools/vt-llrc-probe --matrix`). This
requires macOS 27+: on earlier macOS under Rosetta, VT's internal conversion
produced all-zero chroma, which is why a `rgb_to_nv12` Metal pre-convert
kernel existed historically (see
[the original bug report](apple-feedback-1-lowlatency-bgra-zero-chroma.md)). `ConstantBitRate` is not used: Apple documents
it as incompatible with low-latency rate control (an earlier claim that it was
accepted and then stalled was
[retracted](apple-feedback-2-constantbitrate-pipeline-stall.md)). The
macOS 27+ requirement applies only to this in-process fallback; the arm64
helper path has no such constraint. Either way, encoded NALs
go to the embedded `alvr_server_core` (Rust, C API), which streams over
WiFi or USB to the stock ALVR Quest client v20.14.1; tracking and controller
input return over the same connection and surface as OpenXR actions.

## Repositories

| Piece | Where | Why a fork exists |
|---|---|---|
| oxrsys runtime | `ext/oxrsys` submodule (dingyifei fork, `main`) | The project's own runtime work: input/interaction profiles, session lifecycle, the VideoToolbox encoder, and the embedded-ALVR streaming backend |
| wineopenxr | `ext/wineopenxr` submodule (dingyifei fork, `main`) | `MTLSharedEvent` fence sync, D3D11↔Metal sRGB format mapping, and `XR_KHR_convert_timespec_time`-based time conversion |
| ALVR server core | `ext/ALVR` submodule (dingyifei fork, branch `oxrsys-v20.14.1`) | Reliability patches for embedded in-process use |
| DXMT | Not a submodule: sha256-pinned binaries from the monofunc fork, overlaid onto CrossOver's `lib/dxmt` by `./demo.sh` (stock backed up) | Adds the cross-process texture interop interface (`ImportMTLTexture2D`, `GetFenceSharedEvent`) that stock DXMT lacks |

The ALVR patches also exist as [`patches/alvr-v20.14.1-oxrsys.patch`](../patches/alvr-v20.14.1-oxrsys.patch),
regenerated with
`(cd ext/ALVR && git diff v20.14.1 oxrsys-v20.14.1 > ../../patches/alvr-v20.14.1-oxrsys.patch)`.

## Configuration

- `~/Library/Application Support/OXRSys/oxrsys-runtime.toml` — runtime config
  (`protocol = "alvr"`, `bitrate_mbps`, `encoder_process`, `video_codec`).
  Written once by `./demo.sh setup`, never overwritten. The parser strips
  same-line `#` comments (runtime builds before the 2026-08 fix mis-parsed
  keys with trailing comments — see troubleshooting).
- `encoder_process` (`[streaming]`, default `"auto"`) — `"auto"`/`"native"`
  select the out-of-process arm64 helper (HW HEVC) and are hard-required by
  `run`'s preflight (self-heals by restaging, or fails with a remedy);
  `"inproc"` selects the in-process Rosetta H.264 fallback.
- `video_codec` (`[streaming]`, default `"h265"`; deployed `"auto"`) —
  honored for real on the helper path; `inproc` under Rosetta stays H.264.
- `~/Library/Application Support/OXRSys/alvr/session.json` — the embedded ALVR
  core's session file (not stock ALVR's config directory). Auto-created on
  first run; LAN clients are auto-trusted, so no pairing step.

## Future work

- Bitrate tuning: ~5-6 dropped frames/s at the 80 Mbps cap (first lever: 80→60).
- NV12-in-parent probe (`vt-llrc-probe --gpu --nv12`, adopt only on a ≥5 ms win).
- Frame pacing: ~24 ms p50 `vsync_queue` headroom (owned by the PR#22 pacer thread).
- Quest display-standby freeze: client-side experiments (see troubleshooting).
