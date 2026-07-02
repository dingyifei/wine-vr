# PCVR on Apple Silicon via CrossOver → wineopenxr → oxrsys

Running Windows **D3D11 OpenXR** VR apps on an Apple-Silicon Mac by bridging them, under
CrossOver/Wine, to **oxrsys** — a native-macOS OpenXR runtime that composites and streams to a
Meta Quest. This repo holds the investigation, reproducers, evidence, and patches.

- **Determination & evidence:** [`FINDINGS-oxrsys.md`](FINDINGS-oxrsys.md) (forward path) and
  [`FINDINGS.md`](FINDINGS.md) (why the SteamVR-under-Wine path is dead).
- **Reproducers / tools:** `src/` (native IOSurface probe, Metal & D3D11 OpenXR clients), build/run
  scripts in the repo root. Each script is tagged with a `# ROLE:` line — the current path is
  `run_beatsaber_1294.sh` (PRIMARY launcher) + `run_quest_gate1.sh` (Quest client); `install_bridge.sh`
  is one-time SETUP; the rest are DIAGNOSTIC / BUILD-TOOL investigation artifacts.
- **Upstream-bound patches:** made as separate commits in `ext/oxrsys` and `ext/wineopenxr`
  (each self-contained for a focused PR to monofunc/demonixis).

## Status

- **Gate 0 — GREEN:** IOSurface + `VK_EXT_metal_objects` sharing works cross-process and across the
  Rosetta-x86 ↔ native-arm64 boundary, byte-exact (incl. CrossOver's MoltenVK 1.2.10).
- **Gate 1 — GREEN:** oxrsys streams to a Quest 3 with confirmed 6DoF.
- **D3D11 bridge — WORKING end-to-end:** a Windows D3D11 OpenXR app under CrossOver →
  `wineopenxr` → DXMT zero-copy MTLTexture → oxrsys → H.264 → Quest 3, holding **72 fps** after the
  frame-pacing fix.
- **Beat Saber 1.29.4 — PLAYABLE on Quest 3:** a real Unity-OpenXR game runs end-to-end through the
  bridge with **no real Steam** (Goldberg emulator satisfies DRM; 1.29.4 predates the Meta-account
  gate). Menu navigation and gameplay both work in-headset. Getting there needed four runtime fixes
  (see the patch list below).

The architecture (`monofunc/wineopenxr` + a DXMT interop fork + oxrsys, with the fixes in this repo)
supersedes the original Vulkan+winevulkan plan and targets *real D3D11 PCVR games*, not just Vulkan
hello_xr.

## Improvement potential

### Native-arm64, out-of-process media half (HEVC + headroom)

Measured on this path (flat frame): **H.264 encode is hardware-accelerated even under Rosetta**
(`UsingHardwareAcceleratedVideoEncoder = true`), ~14.8 ms callback = inherent HW-encoder pipeline
latency (not software compute), and throughput holds 72 fps. So the software-fallback worry applies to
**HEVC only** — VideoToolbox HEVC *hardware* encode is unavailable to a Rosetta-translated process,
which is why this path is stuck on H.264.

The improvement is still architectural: share the composited frame from the bridge (`wineopenxr.so`,
Rosetta) as an **IOSurface** to a **separate native-arm64 process** running oxrsys's
compositor/encoder/streamer. Gate 0 proved the cross-arch IOSurface hand-off is byte-exact, so the
primitive is in hand. Native-side benefits: **HW HEVC** (better quality/compression at the same
bitrate than the Rosetta-only H.264), removal of Rosetta per-call overhead, and general headroom for
heavier real-game frames. It is *not* primarily a latency/throughput fix on the current flat-frame
test — H.264 HW already keeps 72 fps there.

### Low-latency rate control — blocked by a Rosetta/VideoToolbox chroma bug (needs native encode)

`kVTVideoEncoderSpecification_EnableLowLatencyRateControl` roughly **halves encode latency and its
jitter** (measured callback ~14.5 ms → ~7.8 ms, p95 ~20 ms → ~9 ms) — a large, real win. It is
**gated off** (`kEnableLowLatencyRC=false` in `VideoEncoder.mm`) because, **under Rosetta**, the
low-latency encode path produces frames with **correct luma but all-zero chroma** → the client renders
green. This was traced conclusively (plane scan of the decoded buffer: `Y[max≈59]` but `Cb/Cr=[0..0]`;
SPS is High-profile 4:2:0, so chroma *should* exist). It survived every app-level fix — full-range
decode, multi-slice access-unit assembly, and explicit BT.709 color primaries/transfer/matrix on both
the input pixel buffers and the compression session — which rules out color *interpretation* and points
at VideoToolbox's low-latency **encode** dropping chroma under x86_64 translation. Same theme as HEVC HW
encode being unavailable under Rosetta: **Rosetta's translation of newer VideoToolbox paths is broken.**

The fix is the **native-arm64 out-of-process encoder** above — running VideoToolbox natively avoids the
translation entirely, and low-latency RC should then work (worth confirming on the Quest / native runtime).
Default rate control works correctly under Rosetta today (correct color, ~16 ms, steady 72 fps after the
pacing fix).

### Smaller items
- Thread QoS (`QOS_CLASS_USER_INTERACTIVE`) on encode/send threads is applied; a dedicated high-QoS
  VT-submit thread (instead of the Metal completion handler) may trim the tail further.
- Encoder tuning on the current path (`encoder_preset = "speed"`, `resolution_scale`) — modest.
- macOS simulator: H.264 decode added for the Rosetta path; upstream a proper codec-negotiated path.
- `wineopenxr`/DXMT: fold the interop-compat fixes (sRGB/typeless format, `PixelFormatView`) upstream.

## Patches made here (candidates for upstream PRs)

**oxrsys** (`ext/oxrsys`): H.264 encode fallback under Rosetta; loopback discovery beacon;
client-liveness watchdog; swapchain `PixelFormatView` for DXMT import; Apple-client H.264 decode +
codec router; absolute-deadline `xrWaitFrame` pacing; live frame-time/FPS plots in the simulator.

_Beat Saber input/boot fixes (this pass):_
- `XR_KHR_convert_timespec_time` extension — Unity's OpenXR plugin hard-requires the Win32
  performance-counter time extension, which `wineopenxr` synthesizes from this; without it
  `xrCreateInstance` fails.
- Emit `XrEventDataInteractionProfileChanged` when the streaming controller profile resolves — this is
  the key fix that makes Unity's Input System bind the real Oculus Touch device (not the KHR
  Simple-Controller fallback), so menu clicks register.
- Report `oculus/touch_controller` for Quest 3 (a profile the app actually binds), and threshold
  float sources (`trigger/value`, `squeeze/value`, `select/value`) to boolean in `GetButtonClick`.
- Stream a distinct aim (pointer) pose alongside grip so menu lasers point correctly.

**wineopenxr** (`ext/wineopenxr`): wait on the DXMT/Metal fence at `xrReleaseSwapchainImage` so the
runtime never composites a pre-render (black) frame; linearize sRGB swapchain formats for DXMT import
(`mtl_srgb_to_linear`); substitute the Win32 `XR_KHR_win32_convert_performance_counter_time` extension
onto the host's `XR_KHR_convert_timespec_time`.
