# RETRACTED — do not submit: "ConstantBitRate accepted, then output callbacks cease"

**Status (2026-07-04): retracted after instrumented re-testing. Not filed with Apple.**

This report originally claimed that setting `kVTCompressionPropertyKey_ConstantBitRate` on a
hardware H.264 `VTCompressionSession` under Rosetta 2 is accepted (`noErr`) and then permanently
stalls the pipeline (output callbacks cease, submitted pixel buffers leak). Preparing the probe
attachment for submission falsified the claim. Keep this file as the record of why.

## What the instrumented probe shows (`tools/vt-llrc-probe --cbr`)

The probe gained a built-in `--cbr` mode that accounts for every callback outcome separately
(real output vs. `kVTEncodeInfo_FrameDropped` vs. error vs. *silent* — no callback at all).
On macOS 26.5.2 / M3 Max, x86_64 under Rosetta, 240 frames per config at 72 fps:

| Config | CBR property set | Result |
|---|---|---|
| classic RC + NV12 + CBR | **accepted** (noErr) | no stall: 240/240 callbacks, 0 dropped, 0 silent |
| classic RC + BGRA + CABAC + 1.5× DataRateLimits + CBR (exact production mirror) | **accepted** (noErr) | no stall: 240/240 callbacks, 0 dropped, 0 silent |
| low-latency RC + NV12 + CBR | **rejected** (-12900 `kVTPropertyNotSupportedErr`) | property never applies |
| low-latency RC + NV12, no CBR (control) | — | 194 output + 46 *reported drops* (over-budget RealTime dropping), 0 silent |

Three findings kill the report:

1. **No stall reproduces in any configuration**, including an exact mirror of the production
   session (classic RC, BGRA IOSurface input, CABAC, MaxKeyFrameInterval, 1.5× DataRateLimits)
   with CBR accepted.
2. **The original "callbacks=34 / 14 never produced output" evidence was misattributed.** Those
   14 frames are `kVTEncodeInfo_FrameDropped` callbacks — ordinary RealTime rate-control drops
   that occur identically *without* CBR (the content runs 122 Mbps against a 42 Mbps budget).
   The old accounting treated a drop callback as "no callback". Drops release their pixel
   buffers; they are not a stall.
3. **The low-latency encoder already rejects CBR with -12900** — exactly the behavior the report
   demanded Apple implement.

## Why the property is wrong for us anyway

The SDK header (`VTCompressionProperties.h`) documents that `ConstantBitRate` **is not compatible
with `DataRateLimits`, `AverageBitRate`, and `VariableBitRate`** — the production experiment set
it on top of AverageBitRate + DataRateLimits, a documented-invalid combination — and that it "is
not supported in all encoders or in all encoder operating modes" with `kVTPropertyNotSupportedErr`
returned when unsupported. In classic mode the accepted property appears to be silently ignored
(output measured 29.3 Mbps against a 42 Mbps constant rate — true CBR pads up). CBR is also the
wrong tool here: it exists for legacy CDN interop, and ALVR's adaptive-bitrate loop handles rate
adaptation. `AverageBitRate` + exact-budget `DataRateLimits` remains the correct configuration.

## What the live 2026-07-03 stall probably was

The production observation (output ceased after a burst, encoder slots leaked, 100% drops) was
real, but it happened in the same working tree that contained the use-after-free fixed in oxrsys
`47dc2a2`: frame-context fields were written *after* `VTCompressionSessionEncodeFrame` handed the
refcon to VT, corrupting the malloc tiny zone and wedging the encode thread — which produces
exactly "callbacks cease and slots leak". With CBR removal bundled into the same round of changes,
the stall was misattributed to CBR.

*The companion report `apple-feedback-1-lowlatency-bgra-zero-chroma.md` is unaffected: its bug
(LL-RC + BGRA → all-zero chroma) reproduces on every run of the probe's default mode.*
