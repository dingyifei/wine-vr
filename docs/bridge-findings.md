# Bridge findings — the working path, gate by gate

Engineering record of the risk-retirement gates behind the demo in the
[README](../README.md): Beat Saber 1.29.4 → CrossOver/DXMT → wineopenxr →
oxrsys → embedded ALVR → Quest 3 at 72 fps and ~79 ms motion-to-photon —
what each gate proved and which fixes were load-bearing. Condensed; the full
transcript is in the git history of `FINDINGS-oxrsys.md` (repo root). The
dead SteamVR-under-Wine path:
[steamvr-blocked](history/steamvr-blocked.md).

## Environment

| Component | Value |
|---|---|
| Host | Apple M3 Max, macOS 26.5.1 |
| CrossOver | 26.2.0 — bottle MoltenVK **1.2.10** (x86_64, Vulkan api 1.2.290) |
| Native arm64 Vulkan | Homebrew MoltenVK **1.4.1** + vulkan-loader 1.4.350 |
| Toolchain | clang 21, mingw-w64 16.1, cmake/ninja/meson + Metal toolchain |

## Gate 0 — IOSurface + MTLSharedEvent, cross-process and cross-arch — GREEN

**Question:** can a cross-process GPU surface — the blocking primitive for Mac
VR — be shared via `VK_EXT_metal_objects`/IOSurface, across the
Rosetta-x86_64 ↔ native-arm64 boundary between the Wine bottle and oxrsys?

**Method:** `src/gate0_iosurf.mm` — Metal fills an IOSurface-backed texture
and signals an `MTLSharedEvent`; Vulkan imports both (`VkImage` + timeline
`VkSemaphore`), waits, copies out, byte-compares. Cross-process mode shares
by global `IOSurfaceID`. Built for arm64 (MoltenVK 1.4.1) *and* for x86_64
against CrossOver's own MoltenVK 1.2.10, run under Rosetta.

**Results** — all byte-exact, 0/16384 mismatched (`evidence/gate0-*.txt`,
local artifacts, not in the repo):

| Test | MoltenVK | Result |
|---|---|---|
| 0a single-process import + MTLSharedEvent→VkSemaphore sync | 1.4.1 arm64 | **PASS** |
| 0a single-process import + sync | **1.2.10 x86_64 (Rosetta)** | **PASS** |
| 0b cross-process IOSurface share | arm64 → arm64 | **PASS** |
| 0b cross-process **+ cross-arch**: arm64 creator → x86_64/Rosetta importer | mixed | **PASS** |
| 0b cross-process + cross-arch: x86_64/Rosetta creator → arm64 importer | mixed | **PASS** |

**Determination:** the zero-copy mechanism works — through the bottle's own
MoltenVK under Rosetta and across the x86↔arm64 boundary in both directions;
no MoltenVK upgrade needed. MTLSharedEvent *sync* was proven single-process
on both arches; cross-process event transport is covered by the OpenXR
swapchain-release protocol. Residual risk: low.

## Gate 1 — oxrsys native → Quest 3 — GREEN (live)

A minimal native Metal OpenXR client (`src/oxrsys_cubes.mm`, full
`xrWaitFrame/Begin/EndFrame` loop) drove oxrsys to a Quest 3 over USB ADB
reverse TCP: 2272×1264 H.265 @ 72 Hz, every frame decoded, render-pose match
100%, client latency ≈11–12 ms (`evidence/gate1-*.txt`, local artifacts, not
in the repo). The headset pose streams back and the app re-renders from it —
6DoF world-locked rendering confirmed visually in-headset, hand tracking
active. Launcher: `scripts/dev/run_quest_gate1.sh`. Fast-motion artifacting
is an oxrsys quality limitation, not a gate blocker.

**Verdict:** the existential dependency — a native macOS OpenXR runtime that
streams to the Quest with tracking — holds on this machine.

## Pivot — monofunc/wineopenxr (D3D11) supersedes the Vulkan plan

Mid-project we found `monofunc/wineopenxr`: a working D3D11
OpenXR→native-runtime bridge for CrossOver, sharing D3D11 swapchain textures
zero-copy as MTLTextures via a DXMT interop fork (`IMTLD3D11InteropDevice`)
— no winevulkan patch, no Wine-from-source build, real D3D11 PCVR games. It
retired the original Gate 2 (Wine source) and Gate 4 (winevulkan) outright.

## Gate 2′ — the D3D11 stack builds

| Component | How | Result |
|---|---|---|
| **monofunc/dxmt** (feature/openxr, interop) | fork + GitHub CI artifact | `d3d11.dll`, `dxgi.dll`, `d3d10core.dll`, `winemetal.dll` + `winemetal.so` (x86_64) |
| **wineopenxr** | local cmake + mingw (Wine headers only) | `wineopenxr.dll` (PE) + `wineopenxr.so` (Mach-O x86_64) |
| **oxrsys-x64 backend** | rebuilt x86_64 with H.264 ported for Rosetta | `liboxrsys-runtime.dylib`, all tests pass under Rosetta |
| **D3D11 test app** | `src/d3d11_clear.cpp` (shader-free clear-color OpenXR client) + cross-built loader | `d3d11_clear.exe` + `libopenxr_loader.dll` |

## Gate 3′ — D3D11 bridge end-to-end — GREEN

Chain: `d3d11_clear.exe` → `libopenxr_loader.dll` → `wineopenxr.dll` →
`wineopenxr.so` → oxrsys-x64, D3D11↔Metal via DXMT interop, in a dedicated
bottle with the DXMT fork overlaid. Headless: wine exit 0, 150 frames,
`ImportMTLTexture2D` succeeded for both eyes (`evidence/bridge-SUCCESS-*.txt`,
local artifact, not in the repo). Compositor pixel readback then proved the
rendered content arrives byte-exact and live — the app's pulsing red,
`snapshot src[0,0] BGRA=25,25,213,255` (`evidence/blackscreen-ROOTCAUSE.txt`,
local artifact, not in the repo).

Seven integration fixes were the actual work:

1. App OpenXR `apiVersion` → 1.1.0 (oxrsys ≤1.1.57 rejects the PE headers'
   1.1.60 with `XR_ERROR_API_VERSION_UNSUPPORTED`).
2. `active_runtime.x86_64.json` must live at `/usr/local/share/openxr/1/` —
   the macOS loader ignores `XR_RUNTIME_JSON` under wine's secure-exec.
3. H.264 under Rosetta (HEVC hardware encode unavailable translated);
   `PreferredVideoCodec()` auto-selects.
4. Request a **non-sRGB** swapchain format: an sRGB host texture trips
   `ImportMTLTexture2D`'s `ORIGINAL_FORMAT` check (DXMT expects
   typeless-parent → linear).
5. oxrsys swapchain MTLTextures need `MTLTextureUsagePixelFormatView` (DXMT
   requires it).
6. Fence sync at `xrReleaseSwapchainImage`: oxrsys snapshots without waiting
   for the producer, and DXMT renders on its own queue → black frames;
   wineopenxr now waits on the DXMT/Metal fence before releasing.
7. Discovery loopback: broadcasts to `255.255.255.255` don't loop back on
   macOS; a `127.0.0.1` beacon was added for the local simulator.

Fixes 4–5 are oxrsys↔DXMT interop findings (reported upstream). Remaining
simulator black screens were a codec mismatch, not the bridge: the simulator
only decoded H.265 while the Rosetta path sends H.264 — an `H264Decoder` +
codec router were added. (The legacy streamer's pacing and watchdog fixes are
in the git history; the ALVR backend below replaced it.)

## VideoToolbox under Rosetta

Hardware H.264 encode **works** under Rosetta
(`UsingHardwareAcceleratedVideoEncoder=true`); only HEVC hardware encode is
unavailable translated. The rest is subtler.

**Low-latency rate control: zero-chroma bug, root-caused and fixed.**
`EnableLowLatencyRateControl` halves encode latency (~14.8 → 7.8 ms p50) but
under Rosetta produced correct luma with all-zero chroma — a green image.
Decoded-plane scans (`Y[max≈59]`, `Cb/Cr=[0..0]`) ruled out color
interpretation; the flag was first gated off. `tools/vt-llrc-probe`
(four-config matrix, {LL-RC on/off} × {BGRA/NV12 input}) then isolated the
failing stage: LL+BGRA = dead chroma, LL+NV12 = healthy. The bug is VT's
*internal* RGB→YCbCr conversion inside the low-latency (`rtvc`) encoder under
Rosetta — undocumented: production VT users all feed 4:2:0. Report:
[apple-feedback-1-lowlatency-bgra-zero-chroma](apple-feedback-1-lowlatency-bgra-zero-chroma.md).
Fix (oxrsys `47dc2a2`): 420v biplanar encoder pool + `rgb_to_nv12` Metal
kernel (BT.709 video-range) on every compose path; LL-RC re-enabled with a
retry-without fallback. Encode p50 **33 → 10.3 ms**, live motion-to-photon
**311 → 79 ms**.

**ConstantBitRate — banned, but the stall claim was retracted (2026-07-04).**
`kVTCompressionPropertyKey_ConstantBitRate` is still never used, but the
original "accepted then stalls the pipeline" claim was **retracted**: an
instrumented probe (`--cbr`, including an exact production-config mirror)
shows classic RC accepts-and-ignores CBR while LL-RC rejects it
(-12900); the "stall" evidence was RealTime frame *drops* miscounted as
missing callbacks, and the live freeze matched the same-day use-after-free
below. The header also documents CBR as incompatible with
`AverageBitRate`/`DataRateLimits`, which we set. Retraction record:
[apple-feedback-2-constantbitrate-pipeline-stall](apple-feedback-2-constantbitrate-pipeline-stall.md).

**Bitrate is not enforced.** VT does not reliably hold
`AverageBitRate`/`DataRateLimits` on hard content in *any* mode under Rosetta
(probe: 77–122 Mbps at a 42 Mbps target on noise). Mitigated with ALVR's
Adaptive bitrate feedback loop, not VT-side caps.

**Rejected properties.** `MaxFrameDelayCount` and
`PrioritizeEncodingSpeedOverQuality` are rejected (-12900) by the LL encoder.

**Use-after-free postmortem.** Frame-context fields were written *after*
`VTCompressionSessionEncodeFrame` hands the refcon to VT; LL-RC makes
callbacks near-synchronous, so the callback (which frees the context) can run
before `EncodeFrame` returns. Rule: all refcon writes precede submission.

## Gate 5 — decision — go with the D3D11 path

The original Vulkan+winevulkan plan is unnecessary: the D3D11 path works
end-to-end and is the real-PCVR-game path. Remaining work is polish, not
architecture.

## Gate 6 — real game (Beat Saber 1.29.4) — PLAYABLE on Quest 3

A real Unity-OpenXR title runs through the bridge in-headset with no real
Steam:

- **Version:** Beat Saber **1.29.4** — the first native-OpenXR build;
  predates the Meta account gate that hard-crashes newer builds
  (`GetXPlatformAccessTokenAsync` timeout → `NullReferenceException` in
  `AppInit`). Pinned depot download: see the [README](../README.md).
- **DRM:** Goldberg Steam emulator satisfies `steam_api64.dll` offline (no
  SteamStub on the exe) — also sidesteps CrossOver's Steam/CEF instability.
- **Launch:** `./demo.sh run` (`run_beatsaber_1294.sh` remains in
  `scripts/dev/` as a superseded reference).
- **Confirmed in-headset:** menu navigation (laser + trigger) and gameplay.

Four runtime fixes were needed beyond the clear-app bridge (all in
`ext/oxrsys`, one paired with `ext/wineopenxr`):

1. **`XR_KHR_convert_timespec_time`** advertised and implemented — Unity
   hard-requires the Win32 perf-counter time extension wineopenxr synthesizes
   from it; without it `xrCreateInstance` fails.
2. **`XrEventDataInteractionProfileChanged`** emitted when the controller
   profile resolves — the key input fix: without it Unity's Input System
   stayed on the KHR Simple-Controller fallback and the menu trigger never
   fired.
3. **Profile + float→bool:** report `oculus/touch_controller`, and threshold
   float sources (`trigger/value`, `squeeze/value`, `select/value`) to
   boolean in `GetButtonClick`.
4. **Aim pose** streamed distinctly from grip so menu lasers point where you
   aim.

Audio and haptics landed later with the ALVR backend, verified live. In-song
pause resolved as a game/Unity limitation on every OpenXR runtime — pause
works via X/A or the Quest system button; see
[menu-button](history/menu-button.md).

## ALVR WiFi backend

oxrsys's built-in streamer was replaced by embedding **`alvr_server_core`
v20.14.1** (its C API) into the runtime, with the **stock ALVR client (v20.14.1, version-matched sideload)**
on the headset. `ext/ALVR` is a submodule pinned to the fork branch
`oxrsys-v20.14.1`, which carries the reliability patches;
`patches/alvr-v20.14.1-oxrsys.patch` mirrors that branch as a reviewable diff
([patches/README](../patches/README.md)).

The first WiFi session ran end-to-end — video, tracking, buttons, haptics
and audio — but with ~2.5 stutter bursts/sec, blur from ALVR's 30 Mbps
default overriding the configured bitrate, and a stream that died on headset
sleep. After three fix rounds: **72 fps, total latency 78–82 ms
(motion-to-photon p50 ≈79 ms), encode p50 10.3 ms, Quest decode 23 ms**
(226 ms at first connect), ~60 Mbps Adaptive bitrate, zero encoder drops.

Reconnect took three distinct fixes:

- A macOS-only ABBA deadlock: the CoreAudio capture callback blocked on the
  session lock while `connection_pipeline` held it across cpal device
  enumeration, which needs the HAL mutex the callback's thread holds — every
  reconnect wedged as a client "error 11" timeout. Fix: a non-blocking
  `is_streaming` for the audio callback.
- Stale `StreamSender` clones pinned the UDP port after disconnect → every
  re-bind failed EADDRINUSE. Fix: a closeable stream writer (`close_writer()`
  on every disconnect flavor + an RAII guard) and a lock-free disconnecting
  flag.
- Headset sleep killed tracking: the tracking/statistics receive loops exited
  permanently on socket error; they now retry.

Smaller patches: socket buffers actually sized (macOS rejects `u32::MAX` with
EINVAL, silently leaving 9216 bytes; now 8 MiB with a halving retry), audio
teardown polling 500 → 50 ms, a leftover `client.wired` entry no longer
starves WiFi discovery, and video-send / manual-IP failures are logged. Full
inventory: [patches/README](../patches/README.md).

Top remaining quality lever is stream resolution (currently 0.75-scale of
client-native), then a native-arm64 out-of-process encoder: running
VideoToolbox natively removes the Rosetta fragility (hardware HEVC, LL-RC
without the NV12 workaround) and reuses Gate 0's cross-arch IOSurface
hand-off.
