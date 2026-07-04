# wineopenxr → oxrsys on Apple Silicon — Risk-Retirement Findings

Companion to `FINDINGS.md` (the dead SteamVR path). This tracks the **forward** path: a Windows PE
OpenXR app (Vulkan binding) under CrossOver bridged to **oxrsys**, a native-macOS OpenXR runtime +
Quest streamer. Gates are filled in as they complete.

## Environment
| Component | Value |
|---|---|
| Host | Apple M3 Max, macOS 26.5.1 |
| CrossOver | 26.2.0 — bottle MoltenVK **1.2.10** (x86_64, Vulkan api 1.2.290) |
| Native arm64 Vulkan | Homebrew MoltenVK **1.4.1** + vulkan-loader 1.4.350 (installed this session) |
| Toolchain | clang 21, mingw-w64 16.1, bison/flex, git; cmake/ninja/meson + Metal toolchain installed per-gate |

---

## Gate 0 — native IOSurface + MTLSharedEvent mechanism probe — ✅ GREEN

**Question:** Can a cross-process GPU surface (the one blocking primitive for Mac VR) be shared via
`VK_EXT_metal_objects` / IOSurface, *including across the Rosetta-x86 ↔ native-arm64 boundary* that
separates the Wine bottle (x86_64) from native oxrsys (arm64)?

**Method:** `src/gate0_iosurf.mm` — Metal creates an IOSurface-backed `MTLTexture`, fills a
deterministic BGRA checkerboard, signals an `MTLSharedEvent`; Vulkan imports the IOSurface as a
`VkImage` (`VkImportMetalIOSurfaceInfoEXT`) and the event as a timeline `VkSemaphore`
(`VkImportMetalSharedEventInfoEXT`), waits, `vkCmdCopyImageToBuffer`, and byte-compares. Cross-process
mode shares the surface by global `IOSurfaceID` (`IOSurfaceIsGlobal`) + `IOSurfaceLookup`. Built for
both arm64 (Homebrew MoltenVK 1.4.1) and x86_64 (linked **directly against CrossOver's MoltenVK 1.2.10**,
the exact lib the bottle uses), so the x86_64 binary runs under Rosetta.

**Results** (all byte-exact, 0/16384 mismatched — `evidence/gate0-SUMMARY.txt`, `gate0a-*.txt`, `gate0b-*.txt`):

| Test | MoltenVK | Result |
|---|---|---|
| 0a single-process, full import + MTLSharedEvent→VkSemaphore sync | 1.4.1 arm64 | **PASS** |
| 0a single-process, full import + sync | **1.2.10 x86_64 (Rosetta)** | **PASS** |
| 0b cross-process IOSurface share | 1.4.1 arm64 → arm64 | **PASS** |
| 0b cross-process **+ cross-arch**: arm64 creator → **x86_64/Rosetta** importer | mixed | **PASS** |
| 0b cross-process + cross-arch: x86_64/Rosetta creator → arm64 importer | mixed | **PASS** |

**Determination:** The IOSurface-through-`VK_EXT_metal_objects` mechanism the zero-copy swapchain
depends on **works**, and critically **works through the bottle's own MoltenVK 1.2.10 under Rosetta and
across the x86↔arm64 boundary in both directions**. No MoltenVK upgrade is needed for the mechanism.
The remaining Part-B unknown is therefore *not* "does Metal/MoltenVK support this" (it does) but solely
**"can winevulkan marshal the import struct chains to the PE app"** — exactly Gate 4.

*Scope note:* cross-process MTLSharedEvent **sync** was proven as a primitive single-process (event↔
semaphore mapping, both arches); full cross-process event sync needs XPC transport of the event handle
(a macOS-IPC detail, not a MoltenVK one) and/or is covered by the OpenXR swapchain-release protocol in
the real path. Residual risk: low.

---

## Gate 1 — oxrsys native → Quest — ✅ GREEN (live, Quest 3)

**Live result** (`evidence/gate1-SUMMARY.txt`, `gate1-cubes-streaming.txt`, `gate1-quest-logcat.txt`):
native cubes app → oxrsys → **Quest 3 over USB ADB reverse TCP**, encoded 2272×1264 H.265 @ 72Hz.
- Quest decodes every frame, **render-pose match 100% (hit=N miss=0)**, **client total latency ≈11–12 ms**
  (decode ~10.5, compositor ~1.0), GPU encode ~0.8 ms.
- **Head tracking round-trips**: headset pose streams back to the Mac app (InputManager pos/rot updating
  as the user moves), app re-renders from the new viewpoint → world-locked rendering. Hand tracking also
  active (26 joints). Determination: **oxrsys streams to the Quest with working tracking on this machine.**

**User-confirmed:** 6DoF head-locked rendering verified visually in the headset. Fast-motion artifacting
observed (expected for video-streamed VR; tuning to `pose_warp` + 80 Mbps + full-res 3040×1680 applied and
reduces but doesn't eliminate it). Treated as a known oxrsys quality limitation (project in active
development), not a gate blocker. **Gate 1 = clear:** the project's existential dependency — a native macOS
OpenXR runtime that streams to the Quest with tracking — holds on this machine.



**Built & verified on this machine (no headset):**
- oxrsys runtime built (commit `f8a2d87`); all 4 ctest suites pass.
- `src/oxrsys_cubes.mm` — minimal native Metal OpenXR client (world-locked cubes, stereo, pose-driven
  projection). Drives oxrsys through a full `xrWaitFrame/Begin/EndFrame` loop with a projection layer.
- Headless run (`evidence/gate1-cubes-headless.txt`): runtime negotiation OK → Metal session →
  1512×1680 stereo swapchains (3 imgs/eye) → session reaches `FOCUSED` → **StreamingServer starts**
  (WiFi 172.20.7.139 ports 9943–9948 + USB ADB TCP), 75% scale → encoded 2272×1264, "waiting for
  headset connection." Sustains a steady frame loop (450+ frames, submit ≈0.01 ms). **The full
  app→runtime→compositor→encoder→streaming chain runs on this machine.**
- Quest client APK built (`app-debug.apk`, 3.4 MB) via gradle + NDK 26.3. `run_quest_gate1.sh`
  installs it, wires `adb reverse` 9944/9945/9946/9948, launches it, and runs the cubes app.

**Done (on hardware):** ran `./run_quest_gate1.sh` on a Quest 3 over USB-C — head-locked cubes confirmed
in-headset, ~11–12 ms latency, 72 fps. (This sub-section predates the live result above; kept for the
build/verify trail.)
## PIVOT (mid-project): monofunc/wineopenxr already exists

Mid-project we found **`monofunc/wineopenxr`** — a working **D3D11** OpenXR→native-runtime bridge for
CrossOver, and oxrsys's author (demonixis) was actively integrating it (issue #4, Beat Saber + Unity XR
in simulator, days ago). It forwards D3D11 OpenXR apps to any x86_64 `XR_KHR_metal_enable` runtime,
sharing D3D11 swapchain textures as MTLTextures **zero-copy** via a **DXMT interop fork**
(`IMTLD3D11InteropDevice`) — **no winevulkan patch, no Wine-from-source build**. This supersedes the
original Gate 2 (Wine source) and Gate 4 (winevulkan) and targets *real D3D11 PCVR games*, not just
Vulkan hello_xr. Decision: build & test that stack (user chose "full solo build").

## Gate 2′ — build the wineopenxr D3D11 stack — ✅ all components built

| Component | How | Result |
|---|---|---|
| **monofunc/dxmt** (feature/openxr, interop) | forked → GitHub CI built LLVM15+Wine+DXMT (~24 min) → downloaded artifact | `d3d11.dll, dxgi.dll, d3d10core.dll, winemetal.dll` + `winemetal.so` (x86_64) |
| **wineopenxr** | local cmake + mingw (Wine *headers* only, no Wine build) | `wineopenxr.dll` (PE) + `wineopenxr.so` (Mach-O x86_64) |
| **oxrsys-x64 backend** | rebuilt x86_64 + **ported H.264** (auto-selected under Rosetta: `RunningUnderRosetta()`→`PreferredVideoCodec()`; edits in `RuntimePlatform`, `CodecSelect.h`, `VideoEncoder.mm`, `StreamingServer.cpp`) | `liboxrsys-runtime.dylib` x86_64, all tests pass under Rosetta |
| **D3D11 test app** | wrote `src/d3d11_clear.cpp` (shader-free clear-color OpenXR D3D11 client) + cross-built Windows OpenXR loader from local SDK | `d3d11_clear.exe` (PE32+) + `libopenxr_loader.dll` |

Note: GitHub git/codeload was heavily throttled this session; worked around via CI artifact download,
winehq-gitlab for wine headers, and writing the D3D11 client locally instead of cloning hello_xr.

## Gate 3′ — D3D11 bridge end-to-end — ✅ GREEN (headless + live on Quest 3)

Installed in dedicated bottle `OpenXRTest` (DXMT fork overlaid globally — see [[crossover-dxmt-fork-overlay]]).
Full chain verified headless (`evidence/bridge-SUCCESS-*.txt`):

`d3d11_clear.exe` (D3D11 OpenXR PE) → `libopenxr_loader.dll` → `wineopenxr.dll` → `wineopenxr.so`
→ oxrsys-x64 ; D3D11↔Metal via **DXMT `IMTLD3D11InteropDevice`**. Result: **wine exit 0, 150 frames
submitted, `ImportMTLTexture2D` succeeded both eyes (3 imgs), oxrsys "Streaming server started,
Session begun."** The D3D11 swapchain textures are shared **zero-copy as MTLTextures** to oxrsys.

**5 fixes were required (the integration work):**
1. App OpenXR `apiVersion` → 1.1.0 (oxrsys ≤1.1.57; PE headers 1.1.60 → `XR_ERROR_API_VERSION_UNSUPPORTED`).
2. `active_runtime.x86_64.json` at **`/usr/local/share/openxr/1/`** (macOS loader ignores `XR_RUNTIME_JSON`
   under wine's secure-exec; only checks that system path).
3. oxrsys **H.264 under Rosetta** (HEVC HW-encode unavailable translated) — `PreferredVideoCodec()`.
4. App requests **non-sRGB** swapchain format (DXMT builds expected format via *typeless parent* → linear;
   sRGB host texture trips `ImportMTLTexture2D`'s `ORIGINAL_FORMAT` mismatch).
5. oxrsys swapchain MTLTextures need **`MTLTextureUsagePixelFormatView`** (DXMT requires it; oxrsys set
   only `RenderTarget|ShaderRead` → "insufficient Metal texture usage").

Fixes #4 & #5 are oxrsys↔DXMT interop-compat findings worth reporting to demonixis/monofunc (issue #4).

**Live display verified end-to-end (`evidence/blackscreen-ROOTCAUSE.txt`):** oxrsys snapshot pixel
readback proves the D3D11 app's rendered content reaches the compositor **byte-exact and live** —
`snapshot src[0,0] BGRA=25,25,213,255` (left-eye red, pulsing with the app's sine wave). **The
D3D11→DXMT→wineopenxr→oxrsys zero-copy path is fully functional.**

Two extra fixes found while getting frames to display:
6. oxrsys→DXMT snapshot **sync**: oxrsys snapshots the swapchain at `xrReleaseSwapchainImage` without
   waiting for the producer's render; the D3D11/DXMT path renders on DXMT's internal queue. Fixed in the
   bridge (wineopenxr waits on the DXMT/Metal fence at release before native release — Codex) + a
   defensive GPU-completion query in the probe.
7. oxrsys discovery: server only broadcast to `255.255.255.255`; macOS doesn't loop that back to a local
   client. Added a **loopback beacon** to `127.0.0.1` (StreamingServer.cpp) so the on-machine simulator discovers it.

**Simulator black screen = codec mismatch, NOT the bridge:** the macOS simulator ships only
`H265Decoder.swift`; oxrsys must send **H.264** under Rosetta → simulator can't decode it. The Quest
client is multi-codec. To view live output: **stream to the Quest** (recommended) or add an H.264 decoder
to the simulator. (Report to demonixis: macOS simulator needs H.264 decode for the CrossOver/Rosetta path.)

## Live display + performance (macOS simulator)

Got the D3D11 bridge output on-screen via the oxrsys macOS simulator and tuned it. Additional fixes
(each a separate commit; see `README.md`):
- **wineopenxr fence sync at release** (Codex): oxrsys snapshots the swapchain at `xrReleaseSwapchainImage`
  without waiting for the producer; the DXMT path renders on DXMT's own queue → black frames. Wait on the
  DXMT/Metal fence at release. (App-side GPU-completion query was a redundant stopgap, since removed.)
- **oxrsys loopback discovery beacon**: server only broadcast to 255.255.255.255; macOS doesn't loop that
  to a local client → simulator never discovered it. Also beacon to 127.0.0.1.
- **Apple client H.264 decode + codec router**: client shipped only `H265Decoder`; Rosetta path sends H.264
  → black. Added `H264Decoder` + `VideoDecoderRouter` (sniffs param-set NAL types).
- **absolute-deadline `xrWaitFrame` pacing**: `sleep_for`-relative pacing accumulated scheduler oversleep
  into drift (65 fps + jitter, worse under Rosetta). Absolute grid + short spin → steady **72 fps**.
- **client-liveness watchdog**: an abruptly-killed UDP client left the server stuck Connected forever
  (never rebroadcast). 3s no-tracking timeout → resume broadcast. **Verified**: kill sim → "no client
  tracking for 3007 ms; resuming broadcast", encoding stops.
- **live frame-time/FPS plots** in the simulator (Swift Charts) for diagnosing pacing.

**Encoder measured HARDWARE-accelerated even under Rosetta** (`UsingHardwareAcceleratedVideoEncoder=true`);
~14.8 ms callback is inherent HW pipeline latency, not SW compute — throughput holds 72 fps, 0 drops.
Only **HEVC** HW encode is unavailable translated (hence H.264). End-to-end ≈30 ms motion-to-photon.

**Low-latency RC investigated + gated off (Rosetta chroma bug):** `EnableLowLatencyRateControl` halves
encode latency/jitter (~14.8→7.8 ms, p95 ~20→9 ms) but **under Rosetta** produces frames with correct
luma and **all-zero chroma** → green. Traced conclusively (decoded-buffer plane scan: `Y[max≈59]`,
`Cb/Cr=[0..0]`; SPS is High-profile 4:2:0). Survived full-range decode, multi-slice access-unit assembly,
and explicit BT.709 tags on input + session — ruling out color interpretation and pinning it on VT's
low-latency **encode** under x86_64 translation. Same theme as HEVC-HW-unavailable-under-Rosetta.

**Low-latency RC root-caused and FIXED via NV12 input (2026-07-03):** the gate-off above is
superseded. `tools/vt-llrc-probe` (x86_64, 4-config matrix {LL-RC on/off} × {BGRA/NV12 input}:
encodes synthetic high-chroma frames, decodes them back in-process, reports per-plane stats) isolated
the failing stage: **LL+BGRA = dead chroma (`Cb/Cr` flat), LL+NV12 = healthy** — same content, same
session properties. The zero-chroma bug lives in VT's *internal* RGB→YCbCr conversion of the
low-latency (`rtvc`) encoder under Rosetta, not in rate control itself; pre-converted 4:2:0 input
bypasses it. Why it's publicly undocumented: production VT users (ffmpeg, Chromium, OBS, Sunshine)
all feed 4:2:0, so nobody exercises the LL encoder's BGRA import path. (Ready-to-file Feedback
Assistant report: `docs/apple-feedback-vt-rosetta.md`.)

**Fix shipped** (oxrsys `47dc2a2`): encoder pool switched to 420v biplanar + `rgb_to_nv12` Metal
kernel (BT.709 video-range) at the tail of every compose path; `EnableLowLatencyRateControl`
re-enabled with a retry-without fallback. Encode total p50 **33 → 10.3 ms**, live motion-to-photon
**311 → 79 ms**.

New Rosetta/VT gotchas found on the way:
- `kVTCompressionPropertyKey_ConstantBitRate` is **accepted** by the session, then **stalls the VT
  pipeline** under Rosetta — output callbacks cease after a few frames and encoder slots leak.
  Never use.
- VT does **not** reliably enforce `AverageBitRate`/`DataRateLimits` on hard content in *any* mode
  under Rosetta (probe: 77–122 Mbps at a 42 Mbps target on noise). Mitigated with ALVR's Adaptive
  bitrate feedback loop rather than VT-side caps.
- `MaxFrameDelayCount` and `PrioritizeEncodingSpeedOverQuality` are rejected (-12900) by the LL
  encoder.

**Use-after-free postmortem:** frame-context fields were written *after*
`VTCompressionSessionEncodeFrame` hands the refcon to VT. LL-RC makes output callbacks
near-synchronous — the callback (which frees the context) can run before `EncodeFrame` even returns —
so the late writes corrupted the malloc tiny zone and wedged the encode thread. Rule: **all refcon
writes must precede submission.**

**Improvement potential (README):** native-arm64 out-of-process media half — runs VideoToolbox natively,
which removes the Rosetta fragility (unlocks HW HEVC *and* working low-latency RC). Reuses Gate 0's
cross-arch IOSurface hand-off.

## Gate 5 — decision — ✅ go (D3D11 path)
The original Vulkan+winevulkan plan is unnecessary: the **D3D11 path works** via monofunc/wineopenxr +
DXMT interop + (patched) oxrsys, end-to-end on this machine — the real-PCVR-game path. Remaining work is
polish + upstreaming the fixes, not architecture.

## Gate 6 — real game (Beat Saber 1.29.4) — ✅ PLAYABLE on Quest 3
A real Unity-OpenXR title runs end-to-end through the bridge, in-headset, with **no real Steam**:
- **Version:** Beat Saber **1.29.4** (first native-OpenXR build; pulled with DepotDownloader, app 620980
  depot 620981 manifest `6291266771922375922`). It predates the Oculus/Meta cross-platform account gate
  that hard-crashes newer builds (`GetXPlatformAccessTokenAsync` timeout → `NullReferenceException` in
  `AppInit`). Launch flow: `run_beatsaber_1294.sh`.
- **DRM:** Goldberg Steam emulator satisfies `steam_api64.dll` offline (the exe has no SteamStub), so no
  real Steam runs — this also sidesteps the CrossOver Steam/CEF instability.
- **Confirmed in-headset:** menu navigation (laser + trigger click) and gameplay both work.

Four runtime fixes were needed beyond the clear-app bridge (all in `ext/oxrsys`, one paired with
`ext/wineopenxr`):
1. **`XR_KHR_convert_timespec_time`** advertised + implemented — Unity OpenXR hard-requires the Win32
   perf-counter time extension, which wineopenxr synthesizes from this; without it `xrCreateInstance` fails.
2. **`XrEventDataInteractionProfileChanged`** emitted when the streaming controller profile resolves — the
   key input fix. Without it Unity's Input System stayed on its KHR Simple-Controller fallback device and
   the menu's `triggerpressed` action never fired; emitting it makes Unity bind the real Oculus Touch device.
3. **Profile + float→bool:** report `oculus/touch_controller` for Quest 3 (a profile the app binds), and
   threshold float sources (`trigger/value`, `squeeze/value`, `select/value`) to boolean in `GetButtonClick`.
4. **Aim pose streamed** distinctly from grip so menu lasers point where you aim.

**Known follow-ups (not bridge-blocking; see project memory `wine-vr-oxrsys-bridge-status`):** streaming
latency/resolution (native-arm64 out-of-process encoder + res-scale), audio streaming (protocol scaffold
exists, unwired), controller haptics (path exists, unverified), and Beat Saber's in-song pause (the `menu`
action is delivered correctly — the game just doesn't act on it; not a bridge issue).
