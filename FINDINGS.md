# PCVR-to-Quest on Apple Silicon under CrossOver/Wine — Findings

> ⚠️ **HISTORICAL.** This documents *why the SteamVR-under-Wine path is blocked* (the shared
> constant-buffer export gap). It is kept as the investigation record. **The working path is the
> D3D11 → wineopenxr → oxrsys bridge — see [`FINDINGS-oxrsys.md`](FINDINGS-oxrsys.md) and the
> [`README`](README.md).** Beat Saber 1.29.4 is now playable on Quest 3 through that bridge, so any
> "recommendation" below (e.g. falling back to VMware Fusion) is **superseded**.

**Date:** 2026-06-26 · **Goal:** Can Windows PCVR (SteamVR/OpenVR/OpenXR) initialize and
stream to a Meta Quest on Apple Silicon under CrossOver/Wine — and if not, what is the exact
blocking capability and the cheapest way around it?

## TL;DR determination

- **(a) MoltenVK provides NO cross-process shared-resource path usable by DXVK — confirmed empirically.**
  winevulkan→MoltenVK on this machine exposes the *names* `VK_KHR_external_memory/_semaphore` but reports
  **`export=0 import=0` (featuresMask 0x0) for every handle type** (OPAQUE_FD, OPAQUE_WIN32, D3D11_TEXTURE,
  D3D11_TEXTURE_KMT, and both semaphore types). DXVK therefore cannot export a shareable handle — its
  `CreateSharedHandle`/`GetSharedHandle` return E_INVALIDARG. The DXVK→MoltenVK path is a hard dead end.
- **(b) The backend that gets furthest is DXMT, by a wide margin — and the real blocker is NOT MoltenVK.**
  DXMT translates D3D11→Metal natively (no Vulkan). It is the **only** backend that can create, export, and
  **re-open across a separate process** a shared `SHARED_NTHANDLE` *texture* (the VR eye-buffer case) — verified
  with two independent processes. Its two remaining gaps: it cannot export a shared *constant buffer* (the exact
  thing SteamVR fails on), and **no backend in CrossOver exposes a keyed mutex** (`IDXGIKeyedMutex` →
  E_NOINTERFACE everywhere), which VR compositors use to synchronize submitted frames.
- **(c) No in-bottle non-SteamVR runtime sidesteps the requirement.** OpenComposite needs an in-bottle OpenXR
  runtime that doesn't exist; VDXR/ALVR are Windows compositors that use the same D3D11 shared-texture +
  keyed-mutex mechanism and inherit the gap. The only true bypass is a **native macOS OpenXR runtime**
  (OpenXR-OSX) — which means running native apps, not arbitrary existing Windows games.
- **(d) Recommendation (for "play existing PCVR games reliably now"): use VMware Fusion.** Its native
  D3D11→Metal driver implements IOSurface-backed shared resources *with* synchronization and SteamVR/ALVR
  initialize there. The CrossOver/Wine path is **closer than expected** (DXMT already does cross-process shared
  textures) but is blocked behind two missing DXMT features — shared buffers and keyed mutex — that no config can
  supply today. See §Recommendation for the decision matrix.

---

## Environment (evidence baseline)

| Component | Value | Source |
|---|---|---|
| Hardware / macOS | Apple **M3 Max**, macOS **26.5.1** (25F80) | `sw_vers`, `sysctl` |
| CrossOver | **26.2.0** (cxoffice-26.2.0rc2, 2026-06-04) | app Info.plist |
| Bottle | `~/Library/Application Support/CrossOver/Bottles/Steam` (win11_64) | cxbottle.conf |
| MoltenVK | **1.2.10**, Vulkan API **1.2.290** as seen in-bottle | dylib `nm`; `vkext.exe` |
| D3D11 backends | D3DMetal (apple_gptk), DXVK, WineD3D, **DXMT** | `lib/{dxvk,dxmt,wine}`, `lib64/apple_gptk` |
| Backend switch | env `CX_GRAPHICS_BACKEND` ∈ {wined3d,dxvk,dxmt,d3dmetal} | CXBT_*.pm, cxcompatdb.so |

Test programs (this repo): `src/shared_repro.cpp` (D3D11 shared-resource probe, single- and
cross-process), `src/vkext.cpp` (winevulkan external-memory enumerator). Built with mingw-w64 14/GCC.

---

## Phase A — MoltenVK capability as seen by Windows-side Vulkan (`evidence/vulkaninfo-bottle.txt`)

```
device: Apple M3 Max (api 1.2.290)
  external device extensions: VK_KHR_external_fence, VK_KHR_external_memory,
                              VK_KHR_external_semaphore, VK_EXT_external_memory_host
  external BUFFER handle-type support (export|import|dedicated):
    OPAQUE_FD          export=0 import=0   (featuresMask=0x0)
    OPAQUE_WIN32       export=0 import=0   (featuresMask=0x0)
    D3D11_TEXTURE      export=0 import=0   (featuresMask=0x0)
    D3D11_TEXTURE_KMT  export=0 import=0   (featuresMask=0x0)
  external SEMAPHORE handle-type support:
    OPAQUE_FD          export=0 import=0   (featuresMask=0x0)
    OPAQUE_WIN32       export=0 import=0   (featuresMask=0x0)
```

Crucially, `VK_KHR_external_memory_fd` and `VK_KHR_external_memory_win32` are **absent**, and even the
present base extensions report zero exportable/importable features. winevulkan bridges D3D11 SHARED_NTHANDLE
to Vulkan only by translating Win32↔opaque-fd; with no fd handle type and zero feature bits, there is nothing
to bridge. This is corroborated by the MoltenVK 1.2.10 binary, which only accepts external-memory handle types
`MTLBUFFER`/`MTLTEXTURE` (intra-process Metal object pointers) — see dylib strings in the session log.
Public MoltenVK changelogs (1.3.x/1.4.x) add `VK_EXT_external_memory_metal` but still no opaque-fd/win32
cross-process handle types.

## Phase B — Isolated shared-resource matrix (`evidence/repro-*.txt`)

All four backends create a D3D11 device (FL 11_0) and *create* resources with the SHARED flags (the misc
flags are silently accepted). The capability difference is entirely in **exporting/importing** a usable handle:

| Capability | wined3d | dxvk | **dxmt** | d3dmetal |
|---|---|---|---|---|
| QI `IDXGIResource1` (buffer/tex) | S_OK | S_OK | S_OK | **E_NOINTERFACE** |
| Constant buffer, legacy `GetSharedHandle` | E_NOTIMPL | E_INVALIDARG | E_INVALIDARG | E_NOTIMPL |
| Constant buffer, `SHARED_NTHANDLE` export | E_NOTIMPL | E_INVALIDARG | E_INVALIDARG | — |
| **Texture `SHARED_NTHANDLE` export** | E_NOTIMPL | E_INVALIDARG | **S_OK** | — |
| **Texture re-open, same process** | — | — | **S_OK** | — |
| **Texture re-open, SEPARATE process** (`OpenSharedResourceByName`) | — | — | **S_OK** ✅ | — |
| `IDXGIKeyedMutex` on shared resource | E_NOINTERFACE | E_NOINTERFACE | **E_NOINTERFACE** | E_NOINTERFACE |

Cross-process proof (DXMT) — two separate `wine` invocations sharing the bottle's wineserver:
```
process A (create-tex): CreateTexture2D(NTHANDLE|KEYEDMUTEX)=S_OK; CreateSharedHandle(named)=S_OK; EXPORTED
process B (open-tex):   OpenSharedResourceByName(tex)=S_OK            <-- genuine cross-process texture share
                        QI IDXGIKeyedMutex (opened-tex)=E_NOINTERFACE <-- but no synchronization primitive
```

**Reading:** DXMT implements real cross-process shared *textures* on Metal (almost certainly IOSurface-backed
via Wine's D3DKMT, no Vulkan/MoltenVK involved) — the exact capability VMware Fusion uses and Parallels lacks.
What's still missing under CrossOver: (1) shared *constant buffers* (fail on every backend, every path), and
(2) keyed-mutex synchronization (absent on every backend).

## Phase C — SteamVR under CrossOver (`evidence/vrcompositor-{dxmt,dxvk}.txt`)

SteamVR 2.16.7 installed and launched in-bottle (`vrstartup.exe`) with the **null driver** (so the compositor
reaches graphics init without a physical HMD). vrserver loads the null HMD fine; the compositor then dies at the
**identical step under both DXMT and DXVK** — the same `VRInitError_Compositor_CreateSharedFrameInfoConstantBuffer`
seen under Parallels:

```
[Info]  - CGraphicsDevice Init...
[Info]  - Getting device factory  /  Getting device adapter
[Info]  - Creating device
[Info]  - Device created                         <-- plain D3D11 device + adapter: OK
[Info]  - Creating constant buffers
[Error] - Failed to create shared frame info constant buffer!   <-- the SHARED constant buffer
[Error] - Failed to init graphics device
[Info]  - Failed to start compositor: VRInitError_Compositor_CreateSharedFrameInfoConstantBuffer
```

Both backends reach "Device created" and fail only at the **shared** constant buffer — exactly matching Phase B
(no CrossOver backend can export a shared *buffer* by any path). DXMT's own translation layer additionally logs
`err: CreateSwapChain: cross-process swapchain not supported yet`, confirming its cross-process surface is partial
(textures yes, swapchain/buffer no).

**`macos_default` branch:** attempted, but Steam did not actually switch SteamVR off `public` — after the download
the appmanifest still read `BetaKey=public` (UserConfig and MountedConfig) and `vrcompositor.exe`/`version.txt`
were byte-for-byte the public build (what downloaded was SteamVR *Workshop* content, not branch binaries). A fresh
run on the current install reproduced the identical `CreateSharedFrameInfoConstantBuffer` failure. Note the
historical `macos_default` branch is the **native-macOS SteamVR 1.x** line — it only matters for running SteamVR
*natively on macOS*, not the Windows build under Wine (which uses D3D11 shared resources regardless of branch), so
it cannot change this outcome. Conclusion: the Wine path is **confirmed-blocked** at the shared constant buffer on
every available backend and branch.

## Phase D — Alternative runtimes
- **OpenComposite** (OpenVR→OpenXR shim): removes SteamVR's compositor but needs an OpenXR runtime present in
  the *bottle*; none exists that can drive a Quest. Does not help on its own.
- **VDXR / ALVR:** VDXR is a Windows OpenXR compositor using the same D3D11 shared-texture mechanism; ALVR is a
  SteamVR driver and requires the SteamVR compositor. Both inherit the keyed-mutex/shared-buffer gap if run
  in-bottle. Neither sidesteps it.
- **Native macOS OpenXR (OpenXR-OSX), WiVRn, Monado:** only a *native* macOS OpenXR runtime avoids the Windows
  shared-resource path entirely. OpenXR-OSX (open-sourced ~May 2026, single-dev, early, some latency) runs on
  Metal natively and ships a Quest thin-client — but it runs *native* apps, not existing Windows SteamVR games.

## Phase E — Bridge feasibility (constant buffer + sync)
The Parallels failure is specifically the *shared frame info constant buffer*. Two facts reshape the bridge
question: (1) the failing object is a small **buffer**, and no CrossOver backend can share a buffer by any path;
(2) the bigger obstacle is the **universally missing keyed mutex**. The right layer to fix is **DXMT, not DXVK**
— DXVK/MoltenVK cannot share anything here, so a DXVK CPU-staging fork is the wrong target (dead end). In DXMT,
which already shares textures cross-process, adding (a) a small CPU/IOSurface-backed shared **buffer** export and
(b) an `IDXGIKeyedMutex` backed by a named cross-process fence is a plausible but **non-trivial code
contribution**, not a config tweak. Verdict: a bridge is *viable in principle in DXMT* but is real engineering;
it is **not** achievable today by configuration or a DXVK hack.

---

## Recommendation

> **Superseded (2026-07):** This matrix predates the working D3D11 → wineopenxr → oxrsys bridge. The
> CrossOver/Wine + DXMT row is no longer "two features away" — the monofunc DXMT fork's zero-copy
> `ImportMTLTexture2D` closed the gap, and Beat Saber 1.29.4 now plays end-to-end on Quest 3 through
> that bridge (no VM needed). See [`FINDINGS-oxrsys.md`](FINDINGS-oxrsys.md). The table below is the
> original SteamVR-era assessment, kept for the record.

| Path | End-to-end today? | Effort | Notes |
|---|---|---|---|
| **VMware Fusion** (native D3D11→Metal) | **Yes** (proven: SteamVR+ALVR init) | Install a VM | Best for *playing existing games now*. IOSurface shared resources **with** sync. |
| CrossOver/Wine + **DXMT** | No | 2 missing DXMT features | Closest Wine option; shares eye-textures already. Blocked on shared-buffer + keyed-mutex. Worth a feature request to the DXMT dev. |
| CrossOver/Wine + DXVK | No (hard dead end) | — | MoltenVK exposes zero external-memory features. Don't pursue. |
| **Native OpenXR-OSX** | Partially | Native, immature | Sidesteps everything but runs native apps, not Windows games. Track for the future. |

**Call:** For the stated goal — reliably play existing Windows PCVR games streamed to a Quest on this M3 Max —
go **VMware Fusion**. Keep DXMT on the radar: it is the only translator on the machine that already does
cross-process GPU texture sharing, so the Wine path is "two features away," and those features live in an
open-source project (DXMT) rather than in the closed SteamVR compositor or in MoltenVK.

### Evidence files
`evidence/vulkaninfo-bottle.txt`, `evidence/repro-{wined3d,dxvk,dxmt,d3dmetal}.txt`,
`evidence/xproc-dxmt-{create,open}.txt`, and (Phase C) `evidence/steamvr-*.txt` + copied SteamVR logs.

---

## Phase F — D3DMetal 4 (GPTK 4) re-test (2026-07-01)

**Question:** GPTK 4 shipped a new D3DMetal. Does **D3DMetal 4** finally expose the *standard* DXGI
sharing path (`IDXGIResource1::CreateSharedHandle` + `OpenSharedResourceByName` + `IDXGIKeyedMutex`)?
If so, the wineopenxr bridge could run on **stock GPTK 4** and drop the monofunc DXMT fork entirely.
Phase B tested D3DMetal **3.0** (still the CrossOver 26.2 default); this re-runs the identical probe
against **D3DMetal 4.0b1**.

**Setup:** GPTK 4.0 beta 1 evaluation environment (`Game_Porting_Toolkit_4.0_beta_1.dmg`). Overlaid
per Apple's Read Me — wholesale `ditto redist/lib/` replacing both `external` and `wine` in
`CrossOver.app/.../lib64/apple_gptk` (stock preserved, then restored). Confirmed
`D3DMetal.framework` = `4.0b1` loaded. Same `src/shared_repro.cpp` probe, `CX_GRAPHICS_BACKEND=d3dmetal`,
single- and cross-process.
*Gotcha:* a **partial** per-dll overlay (framework only, or framework + a few dlls) crashes the process
silently (exit 66, zero output) — the `external` + `wine` set is version-coupled and must be swapped
wholesale via `ditto`, exactly as Apple documents.

| Capability (the 3 that would matter) | D3DMetal **3.0** | D3DMetal **4.0b1** | Change? |
|---|---|---|---|
| `QI IDXGIResource1` (texture) | E_NOINTERFACE | **E_NOINTERFACE** | none |
| Texture `SHARED_NTHANDLE` cross-proc `OpenSharedResourceByName` | — (unreachable) | **E_NOTIMPL** | none |
| `QI IDXGIKeyedMutex` | E_NOINTERFACE | **E_NOINTERFACE** | none |

Legacy `GetSharedHandle` = E_NOTIMPL, `QI IDXGIResource1` (constant buffer) = E_NOINTERFACE — also
unchanged. D3DMetal 4 creates the device (FL 11_0) and resources fine but exposes **no** shared-resource
interface of any kind, exactly like 3.0. Evidence: `evidence/repro-d3dmetal-gptk4.txt`,
`evidence/xproc-d3dmetal-gptk4-{create,open}.txt`.

**Verdict:** GPTK 4 / D3DMetal 4 does **not** help this bridge. Its real gains (DX12→Metal 4,
framebuffer fetch, MetalFX, DXR) are orthogonal to cross-process GPU resource sharing, which D3DMetal
still does not implement — and being closed-source it cannot be forked to add it. **Stay on the monofunc
DXMT fork** (`ImportMTLTexture2D` / `GetFenceSharedEvent`), which remains the only D3D11→Metal translator
on this machine that shares textures cross-process. Question closed.
