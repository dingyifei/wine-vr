# Why SteamVR under Wine is blocked on Apple Silicon

> **HISTORICAL RECORD.** This is the investigation that established *why the SteamVR-under-Wine
> path is impossible* on Apple Silicon today: no CrossOver graphics backend can export a shared
> D3D11 constant buffer, and none exposes a keyed mutex. Everything prescriptive here is
> **superseded** — the working path is D3D11 → wineopenxr → oxrsys → embedded ALVR, documented in
> [bridge findings](../bridge-findings.md) and the [README](../../README.md). Beat Saber 1.29.4
> now plays end-to-end on Quest 3 through that bridge, so the VMware Fusion recommendation below
> is obsolete. Full transcript in the git history of `FINDINGS.md`.

**Date:** 2026-06-26 (Phase F re-test: 2026-07-01)
**Question:** can Windows PCVR (SteamVR/OpenVR/OpenXR) initialize and stream to a Meta Quest on
Apple Silicon under CrossOver/Wine — and if not, what is the exact blocking capability and the
cheapest way around it?

## TL;DR determination

- **(a) MoltenVK provides no cross-process shared-resource path usable by DXVK — confirmed
  empirically.** winevulkan→MoltenVK exposes the *names* `VK_KHR_external_memory/_semaphore` but
  reports **`export=0 import=0` (featuresMask 0x0) for every handle type** (OPAQUE_FD,
  OPAQUE_WIN32, D3D11_TEXTURE, D3D11_TEXTURE_KMT, and both semaphore types). DXVK therefore cannot
  export a shareable handle — `CreateSharedHandle`/`GetSharedHandle` return E_INVALIDARG. The
  DXVK→MoltenVK path is a hard dead end.
- **(b) The backend that gets furthest is DXMT, by a wide margin — and the real blocker is not
  MoltenVK.** DXMT translates D3D11→Metal natively (no Vulkan). It is the **only** backend that
  can create, export, and **re-open across a separate process** a shared `SHARED_NTHANDLE`
  *texture* (the VR eye-buffer case) — verified with two independent processes. Its two remaining
  gaps: it cannot export a shared *constant buffer* (the exact thing SteamVR fails on), and **no
  backend in CrossOver exposes a keyed mutex** (`IDXGIKeyedMutex` → E_NOINTERFACE everywhere),
  which VR compositors use to synchronize submitted frames.
- **(c) No in-bottle non-SteamVR runtime sidesteps the requirement.** OpenComposite needs an
  in-bottle OpenXR runtime that doesn't exist; VDXR/ALVR are Windows compositors that use the same
  D3D11 shared-texture + keyed-mutex mechanism and inherit the gap. The only true bypass is a
  **native macOS OpenXR runtime** — which at the time meant running native apps, not arbitrary
  existing Windows games.
- **(d) Recommendation at the time: use VMware Fusion**, whose native D3D11→Metal driver
  implements IOSurface-backed shared resources *with* synchronization (SteamVR/ALVR initialize
  there). **Superseded** — the bridge in this repo made the VM unnecessary; see the banner above.

## Environment (evidence baseline)

| Component | Value | Source |
|---|---|---|
| Hardware / macOS | Apple **M3 Max**, macOS **26.5.1** (25F80) | `sw_vers`, `sysctl` |
| CrossOver | **26.2.0** (cxoffice-26.2.0rc2, 2026-06-04) | app Info.plist |
| Bottle | `~/Library/Application Support/CrossOver/Bottles/Steam` (win11_64) | cxbottle.conf |
| MoltenVK | **1.2.10**, Vulkan API **1.2.290** as seen in-bottle | dylib `nm`; `vkext.exe` |
| D3D11 backends | D3DMetal (apple_gptk), DXVK, WineD3D, **DXMT** | `lib/{dxvk,dxmt,wine}`, `lib64/apple_gptk` |
| Backend switch | env `CX_GRAPHICS_BACKEND` ∈ {wined3d,dxvk,dxmt,d3dmetal} | CXBT_*.pm, cxcompatdb.so |

Test programs (this repo): [`src/shared_repro.cpp`](../../src/shared_repro.cpp) (D3D11
shared-resource probe, single- and cross-process) and [`src/vkext.cpp`](../../src/vkext.cpp)
(winevulkan external-memory enumerator), built with mingw-w64 14.

## Phase A — MoltenVK external-memory capability

- Every external handle type winevulkan can see reports `export=0 import=0` (featuresMask 0x0),
  and `VK_KHR_external_memory_fd`/`_win32` are absent entirely. winevulkan bridges D3D11
  `SHARED_NTHANDLE` to Vulkan only by translating Win32↔opaque-fd handles, so with no fd handle
  type and zero feature bits there is nothing to bridge.
- The MoltenVK 1.2.10 binary only accepts handle types `MTLBUFFER`/`MTLTEXTURE` — intra-process
  Metal object pointers. Public 1.3.x/1.4.x changelogs add `VK_EXT_external_memory_metal` but
  still no opaque-fd/win32 cross-process handle type.
- Probe output: `evidence/vulkaninfo-bottle.txt` (local artifact, not in the repo).

## Phase B — shared-resource capability matrix

All four backends create a D3D11 device (FL 11_0) and silently accept the SHARED misc flags at
resource creation. The capability difference is entirely in **exporting/importing** a usable
handle:

| Capability | wined3d | dxvk | **dxmt** | d3dmetal |
|---|---|---|---|---|
| QI `IDXGIResource1` (buffer/tex) | S_OK | S_OK | S_OK | **E_NOINTERFACE** |
| Constant buffer, legacy `GetSharedHandle` | E_NOTIMPL | E_INVALIDARG | E_INVALIDARG | E_NOTIMPL |
| Constant buffer, `SHARED_NTHANDLE` export | E_NOTIMPL | E_INVALIDARG | E_INVALIDARG | — |
| **Texture `SHARED_NTHANDLE` export** | E_NOTIMPL | E_INVALIDARG | **S_OK** | — |
| **Texture re-open, same process** | — | — | **S_OK** | — |
| **Texture re-open, SEPARATE process** (`OpenSharedResourceByName`) | — | — | **S_OK** | — |
| `IDXGIKeyedMutex` on shared resource | E_NOINTERFACE | E_NOINTERFACE | **E_NOINTERFACE** | E_NOINTERFACE |

Cross-process proof (DXMT) — two separate `wine` invocations sharing the bottle's wineserver:

```
process A (create-tex): CreateTexture2D(NTHANDLE|KEYEDMUTEX)=S_OK; CreateSharedHandle(named)=S_OK; EXPORTED
process B (open-tex):   OpenSharedResourceByName(tex)=S_OK            <-- genuine cross-process texture share
                        QI IDXGIKeyedMutex (opened-tex)=E_NOINTERFACE <-- but no synchronization primitive
```

**Reading:** DXMT implements real cross-process shared *textures* on Metal (almost certainly
IOSurface-backed via Wine's D3DKMT, no Vulkan involved). Still missing under CrossOver: shared
*constant buffers* (fail on every backend, every path) and keyed-mutex synchronization (absent on
every backend).

## Phase C — SteamVR under CrossOver

- SteamVR 2.16.7 launched in-bottle with the **null driver** so the compositor reaches graphics
  init without a physical HMD. vrserver loads the null HMD fine; the compositor creates a plain
  D3D11 device, then dies at "Failed to create shared frame info constant buffer" →
  `VRInitError_Compositor_CreateSharedFrameInfoConstantBuffer` — the **identical step under DXMT
  and DXVK**, and the same error seen under Parallels.
- This matches Phase B exactly: no CrossOver backend can export a shared *buffer* by any path.
  DXMT additionally logs `CreateSwapChain: cross-process swapchain not supported yet`.
- The `macos_default` branch was attempted but Steam never actually switched off `public` (the
  appmanifest kept `BetaKey=public`; binaries were byte-identical) — and that branch is the
  native-macOS SteamVR 1.x line anyway, irrelevant to the Windows build's D3D11 shared-resource
  path. **Confirmed blocked on every available backend and branch.**

## Phase D — alternative runtimes

- **OpenComposite** removes SteamVR's compositor but needs an in-bottle OpenXR runtime that can
  drive a Quest; none existed.
- **VDXR / ALVR in-bottle** use the same D3D11 shared-texture + keyed-mutex mechanism (ALVR is a
  SteamVR driver besides) and inherit the gap.
- **Native macOS OpenXR (OpenXR-OSX, WiVRn, Monado):** only a native runtime avoids the Windows
  shared-resource path — but runs native apps, not existing Windows SteamVR games.

## Phase E — bridge feasibility

- The failing object is a small shared **buffer**; the bigger obstacle is the universally missing
  **keyed mutex**.
- The right layer to fix is **DXMT** (already shares textures cross-process), not DXVK — the
  MoltenVK route can share nothing, so a DXVK CPU-staging fork is the wrong target.
- Adding an IOSurface-backed shared-buffer export plus an `IDXGIKeyedMutex` backed by a
  cross-process fence was judged plausible but **real engineering, not a config tweak**. This is
  essentially the road later taken — the monofunc DXMT fork (`ImportMTLTexture2D` /
  `GetFenceSharedEvent`) plus wineopenxr sidesteps the SteamVR compositor entirely.

## Recommendation (superseded)

The original decision matrix (VMware Fusion "works today", CrossOver+DXMT "two features away",
DXVK dead end, OpenXR-OSX for the future) predates the working bridge and is kept only in the git
history of `FINDINGS.md`; the DXMT-fork + wineopenxr + oxrsys path closed the gap with no VM —
see [bridge findings](../bridge-findings.md).

## Phase F — D3DMetal 4 (GPTK 4) re-test (2026-07-01)

GPTK 4 shipped a new D3DMetal; if **D3DMetal 4.0b1** exposed the standard DXGI sharing path, the
bridge could run on stock GPTK and drop the DXMT fork. Overlaid per Apple's Read Me (the
`external` + `wine` set must be swapped wholesale via `ditto`; a partial overlay crashes silently
with exit 66) and re-ran the identical probe:

| Capability (the 3 that would matter) | D3DMetal **3.0** | D3DMetal **4.0b1** | Change? |
|---|---|---|---|
| `QI IDXGIResource1` (texture) | E_NOINTERFACE | **E_NOINTERFACE** | none |
| Texture `SHARED_NTHANDLE` cross-proc `OpenSharedResourceByName` | — (unreachable) | **E_NOTIMPL** | none |
| `QI IDXGIKeyedMutex` | E_NOINTERFACE | **E_NOINTERFACE** | none |

**Verdict:** D3DMetal 4 creates devices and resources fine but exposes **no** shared-resource
interface of any kind, exactly like 3.0 — and being closed-source it cannot be forked to add one.
Its real gains (DX12→Metal 4, MetalFX, DXR) are orthogonal. The monofunc DXMT fork remains the
only D3D11→Metal translator on this machine that shares textures cross-process. Question closed.

## Evidence

`evidence/vulkaninfo-bottle.txt`, `evidence/repro-{wined3d,dxvk,dxmt,d3dmetal}.txt`,
`evidence/xproc-dxmt-{create,open}.txt`, `evidence/steamvr-*.txt`,
`evidence/repro-d3dmetal-gptk4.txt`, `evidence/xproc-d3dmetal-gptk4-{create,open}.txt`
(all local artifacts, not in the repo).
