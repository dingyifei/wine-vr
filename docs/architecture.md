# Architecture

How a Beat Saber frame gets from a Windows x64 process on an Apple Silicon Mac
into a Quest 3 headset, and which repository owns each hop. For setup and
usage, see the [README](../README.md).

## Frame path

```
Beat Saber.exe (x64, CrossOver/Wine, Rosetta)
  └─ openxr_loader.dll (Unity's stock loader)
      └─ wineopenxr.dll (PE) ⇄ wineopenxr.so (unix side, same process)
            │  D3D11 → Metal via DXMT; swapchain images imported as MTLTextures (zero-copy)
            └─ oxrsys runtime (in-process dylib, x86_64 under Rosetta)
                  └─ rgb_to_nv12 Metal kernel → VideoToolbox H.264 low-latency encode
                        └─ embedded alvr_server_core ── WiFi ──► stock ALVR client (Quest 3)
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

On `xrEndFrame`, the runtime composites layers into a BGRA target, converts it
to BT.709 video-range NV12 with the `rgb_to_nv12` Metal compute kernel (see
[why NV12 input matters](apple-feedback-1-lowlatency-bgra-zero-chroma.md)),
and encodes with a VideoToolbox H.264 session using
`EnableLowLatencyRateControl`. `ConstantBitRate` is not used: Apple documents
it as incompatible with low-latency rate control (an earlier claim that it was
accepted and then stalled was
[retracted](apple-feedback-2-constantbitrate-pipeline-stall.md)). Encoded NALs
go to the embedded `alvr_server_core` (Rust, C API), which streams over WiFi
to the stock ALVR Quest client v20.14.1; tracking and controller input return
over the same connection and surface as OpenXR actions.

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
  (`protocol = "alvr"`, `bitrate_mbps`). Written once by `./demo.sh setup`,
  never overwritten.
- `~/Library/Application Support/OXRSys/alvr/session.json` — the embedded ALVR
  core's session file (not stock ALVR's config directory). Auto-created on
  first run; LAN clients are auto-trusted, so no pairing step.

## Future work

- Native arm64 out-of-process encoder: moves encoding out of the Rosetta
  process, unlocking HEVC and lower latency.
- 1:1 resolution.
- 72 fps pacing refinements.
