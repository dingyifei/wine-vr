# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Beat Saber 1.29.4 (Windows x64) running under CrossOver/Wine on Apple Silicon, streamed to a Quest 3 over WiFi with the stock ALVR client:

```
Beat Saber.exe (x64, Wine, Rosetta)
  └─ openxr_loader.dll → wineopenxr.dll (PE) ⇄ wineopenxr.so (unix, same process)
        │  D3D11 → Metal via DXMT fork; swapchain images imported as MTLTextures (zero-copy)
        └─ oxrsys runtime (in-process x86_64 dylib)
              └─ rgb_to_nv12 Metal kernel → VideoToolbox H.264 LL encode
                    └─ embedded alvr_server_core (Rust, C API) ── WiFi ──► ALVR client v20.14.1 (Quest 3)
```

This repo itself is small (~41 tracked files): the demo pipeline (`demo.sh` + `scripts/demo/`), docs, investigation-era probes (`src/`, `tools/`, `scripts/dev/`), and one patch mirror. **All runtime code lives in three submodule forks under `ext/`:**

| Submodule | Fork/branch | Owns |
|---|---|---|
| `ext/oxrsys` | dingyifei/oxrsys, `main` | The OpenXR runtime: session lifecycle, input/interaction profiles, VideoToolbox encoder, both streaming backends |
| `ext/wineopenxr` | dingyifei/wineopenxr, `main` | PE↔unix bridge: MTLSharedEvent fence sync, D3D11↔Metal sRGB mapping, timespec time conversion |
| `ext/ALVR` | dingyifei/ALVR, `oxrsys-v20.14.1` | ALVR v20.14.1 + reliability patches for embedded in-process use |

DXMT is deliberately **not** a submodule: sha256-pinned binaries from the monofunc fork (adds `ImportMTLTexture2D`/`GetFenceSharedEvent`, which stock DXMT lacks) are overlaid onto CrossOver's `lib/dxmt` by install; stock is backed up once to `$CX/lib/dxmt.stock-backup`.

`ext/oxrsys` has its own `AGENTS.md` with repo-specific rules (build+test before claiming success, versions only in `config/OXRSysVersion.xcconfig`, MPL-2.0 headers, never link the Vulkan loader, `Session::EndFrame()` stays non-blocking). Read it before working there.

## Commands

The demo pipeline (`demo.sh`, zsh) is the entry point; there is no Makefile or CI:

```sh
./demo.sh doctor --bottle <name>    # ~30 preflight checks, each FAIL prints a remedy; exit code = FAIL count
./demo.sh setup                     # submodules + sha256-pinned binaries + runtime config (idempotent, no sudo)
./demo.sh build                     # oxrsys (x86_64 + ALVR core) and wineopenxr
./demo.sh install --bottle <name>   # bridge into CrossOver + bottle + host loader (the ONLY sudo stage)
./demo.sh run --bottle <name>       # launch Beat Saber (also: --bs-dir, --no-audio, --no-dashboard, --verbose)
./demo.sh stop --bottle <name>      # kill game + wineserver, check ports/audio
```

Flags mirror env vars: `WINEVR_BOTTLE`, `WINEVR_BS_DIR`, `WINEVR_NO_AUDIO`, `WINEVR_NO_DASHBOARD`, `WINEVR_VERBOSE`. Stage scripts in `scripts/demo/` are **sourced** by the dispatcher and depend on `lib.sh` globals — they cannot run standalone. `lib.sh` is the single source of truth for paths, sha256 pins, and helpers.

What `build` actually runs (useful for iterating on one component):

```sh
# oxrsys — Debug x86_64 is the live-verified config (game runs under Rosetta, runtime loads in-process)
cmake -S ext/oxrsys -B ext/oxrsys/build-x64 -G Ninja -DCMAKE_BUILD_TYPE=Debug \
      -DCMAKE_OSX_ARCHITECTURES=x86_64 -DOXRSYS_ENABLE_ALVR=ON
cmake --build ext/oxrsys/build-x64 -j8

# wineopenxr — PE dll via mingw + unix .so
cmake -S ext/wineopenxr -B ext/wineopenxr/build && cmake --build ext/wineopenxr/build -j8
```

### Tests (oxrsys)

```sh
cd ext/oxrsys
ctest --test-dir build --output-on-failure          # all suites (build/ = arm64 dev tree, see below)
ctest --test-dir build -R oxrsys_alvr_session_config  # one ctest entry
./build/oxrsys_runtime_tests "[alvr-session]"       # Catch2 direct: tag subset or test name
```

Caution: ctest runs with an isolated HOME under `build/test-env`; running a test binary directly uses your **real** `~/Library/Application Support/OXRSys`.

### Other

```sh
# Regenerate the reviewable ALVR patch mirror after ANY change to the ext/ALVR branch
(cd ext/ALVR && git diff v20.14.1 oxrsys-v20.14.1 > ../../patches/alvr-v20.14.1-oxrsys.patch)

# Still-useful diagnostics (most of scripts/dev/ is closed-investigation era)
scripts/dev/run_d3d11_bridge.sh    # minimal D3D11 app through the bridge, no Beat Saber
scripts/dev/run_quest_gate1.sh     # streaming path without Wine (needs arm64 ext/oxrsys/build)
```

## Architecture notes

- **Runtime discovery is the one non-obvious hop.** Wine's secure-exec ignores `XR_RUNTIME_JSON`; the host OpenXR loader finds oxrsys only via the root-owned `/usr/local/share/openxr/1/active_runtime.x86_64.json` (written by `install`), which embeds the **absolute path** to `ext/oxrsys/build-x64/runtime/liboxrsys-runtime.dylib` — moving the repo breaks routing.
- **`install` touches 4 layers:** global DXMT overlay in `$CX/lib/dxmt`, global wineopenxr in `$CX/lib/wine`, per-bottle (system32 dll + `drive_c/openxr/` manifest + `HKLM\Software\Khronos\OpenXR\1` registry key), and the host manifest above. A CrossOver update silently reverts the global overlays — `doctor` and `run`'s preflight both catch this; the fix is always re-running `install`.
- **Two oxrsys build trees coexist:** `ext/oxrsys/build` (arm64, native dev/tests, used by Gate 1 tooling) and `ext/oxrsys/build-x64` (x86_64 + ALVR, what the pipeline installs from). Don't conflate them; both read the same user config.
- **ALVR embedding:** oxrsys's CMake (`cmake/AlvrServerCore.cmake`) cargo-builds `libalvr_server_core.dylib` from `ext/ALVR` and stages it next to the runtime dylib. It needs the **rustup** stable toolchain with the `x86_64-apple-darwin` target — Homebrew cargo lacks the cross-target std (override: `-DOXRSYS_CARGO_BIN_DIR`). If rustup or the ALVR checkout is missing, a bare cmake configure silently disables the backend with only a WARNING (`demo.sh build` fails earlier, loudly).
- **The ALVR C API header** (`ext/oxrsys/runtime/src/alvr/alvr_server_core.h`) is hand-written and pinned to ALVR v20.14.1 commit a9f6542; the upstream C API changed after that tag, so bumping the ALVR pin without re-verifying against `c_api.rs` produces silent ABI breakage. `oxrsys-runtime.toml` is the single source of truth: it's synced into ALVR's `session.json` **before** `alvr_initialize` (server_core reads it once at init).
- **Streaming backends:** two implementations behind `IStreamingBackend` in oxrsys, selected by `protocol = "alvr" | "oxrsys"` in the runtime config. `alvr` is the demo path (stock Quest client); `oxrsys` is the legacy USB/adb-reverse protocol. Preserve `StopForProcessExit()` — `alvr_shutdown()` hangs at process exit by design.
- **Config/state:** `~/Library/Application Support/OXRSys/` holds `oxrsys-runtime.toml` (written once by `setup`, never overwritten; `doctor` FAILs if protocol ≠ alvr), `alvr/session.json` (auto-created, LAN auto-trust; delete it to clear a stale IP pin after a DHCP change), and runtime logs. The oxrsys 1.3.0 merge added `[streaming]` keys (`render_device`, `video_codec`, `encoder_10bit`, `client_sharpening`, `app_alpha_blend_passthrough`); deployed write-once configs that predate them fall back to code defaults identical to the template, so no migration is needed — and `render_device` only affects the legacy oxrsys protocol, not the ALVR session template.

## Cross-repo workflow

Changes to `ext/*` must land on the fork branch, be **pushed to the dingyifei fork first**, then the submodule pointer bump committed in wine-vr — otherwise a fresh `setup` can't fetch the pinned SHA. For `ext/ALVR` additionally regenerate `patches/alvr-v20.14.1-oxrsys.patch` (the branch IS the patch set, already applied — never apply the patch file to the submodule; `setup`/`doctor` sanity-check the pin by grepping for `is_streaming_nonblocking`).

When adding a new requirement to the pipeline, add a `doctor` section with a one-line remedy string, and a matching hard preflight in `run.sh` if it's launch-critical (fresh bottles / CrossOver updates pass machine-global checks yet launch with no VR).

## Constraints that look wrong but aren't

- **x86_64 is load-bearing everywhere.** The whole Wine process runs under Rosetta; oxrsys, the ALVR core, and the VideoToolbox probes are all deliberately x86_64. The VT chroma bug does not reproduce on arm64. Debug is the live-verified build type.
- **VideoToolbox under Rosetta:** H.264 only (no hardware HEVC); the LL-RC encoder must be fed NV12 (BGRA input → all-zero chroma, green video); LL-RC rejects `ConstantBitRate` with -12900. All frame-context writes must happen **before** `VTCompressionSessionEncodeFrame` — LL-RC callbacks are near-synchronous (use-after-free postmortem, oxrsys 47dc2a2). Since the oxrsys 1.3.0 merge, codec choice is negotiated with the client (`CodecSelect.h`), but the Rosetta constraint filters every rung — H.264 is always what the demo path encodes, regardless of the `video_codec` config.
- **Beat Saber must be exactly 1.29.4** — first native-OpenXR build, and newer builds hard-crash on the Meta account gate. DRM is satisfied offline by Goldberg (`run` swaps `steam_api64.dll`); no real Steam ever runs.
- Interop traps in the bridge: non-sRGB swapchain format only (sRGB trips `ImportMTLTexture2D`), app OpenXR apiVersion 1.1.0, swapchain textures need `MTLTextureUsagePixelFormatView`.
- The zsh scripts use `print -r` deliberately (echo mangles backslashes in Windows paths) and zsh array semantics — keep both when editing.

## Closed investigations — do not reopen

- **Left menu button can't pause** — game/Unity limitation on *every* OpenXR runtime since the 1.29.4 port; X/A and the Quest system button work. `docs/history/menu-button.md`.
- **SteamVR under Wine** — impossible on Apple Silicon (no backend exposes shared constant buffers + keyed mutex). `docs/history/steamvr-blocked.md`.
- **The CBR pipeline-stall claim** — retracted 2026-07-04 (misattributed use-after-free). `docs/apple-feedback-2-*.md`.
- **DXVK/MoltenVK external-memory route** — export=0/import=0, dead end.

## Conventions

- Demo runs log to `logs/`; dev/diagnostic scripts write to `evidence/`. Both are gitignored on purpose — docs cite `evidence/` files by name as local artifacts, so don't delete that directory.
- `.gitignore` blanket-ignores binaries (`*.dll`, `*.dylib`, `build/`, `third_party/`, `ext/dxmt-artifacts/`); a root-level `BeatSaberVersion.txt` is expected runtime junk.
- Setup/onboarding and user-facing knobs are documented in `README.md`; the frame path in `docs/architecture.md`; failure modes in `docs/troubleshooting.md`. Keep those current when behavior changes rather than duplicating them here.
