# Feedback Assistant report — VideoToolbox low-latency H.264 encoder emits all-zero chroma for BGRA input under Rosetta 2

**Title:** VideoToolbox low-latency H.264 encoder (EnableLowLatencyRateControl) emits all-zero
chroma planes for BGRA input in x86_64 (Rosetta 2) processes on Apple Silicon

**Area:** VideoToolbox
**Reproducibility:** Always
**macOS version:** macOS 26.5.2 (build 25F84)
**Hardware:** Apple Silicon Mac (observed on M3 Max); calling process running as x86_64 under Rosetta 2

## Summary

With `kVTVideoEncoderSpecification_EnableLowLatencyRateControl` in the encoder specification (the
`rtvc` hardware H.264 encoder) and `kCVPixelFormatType_32BGRA` input pixel buffers, the encoded
bitstream has correct luma but **all-zero chroma planes** — decoded output is uniformly
green-tinted. Feeding the same content pre-converted to NV12
(`kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange`) through an identical session produces correct
output, so the defect is isolated to the low-latency encoder's **internal RGB→YCbCr conversion**,
not rate control or the bitstream path. The bug does **not** reproduce in a native arm64 process.

This is likely invisible to most software because mainstream VideoToolbox clients (ffmpeg,
Chromium, OBS, Sunshine) feed 4:2:0 input; it surfaces for any translated x86_64 app (e.g.
Wine/CrossOver-hosted game streaming) that feeds RGB.

## Repro tool

Self-contained probe (single ObjC++ file, no dependencies): `tools/vt-llrc-probe/main.mm` from the
wine-vr project (file can be attached directly to this report). It encodes 48 synthetic
high-chroma 2496×1312 frames at 72 fps / 42 Mbps through the HW H.264 encoder under a 4-config
matrix {low-latency RC on/off} × {BGRA/NV12 input}, decodes the bitstream back in-process, and
prints per-plane min/max/mean statistics.

Build (must be x86_64 so it runs under Rosetta on Apple Silicon):

```
clang++ -arch x86_64 -std=c++17 -fobjc-arc -O2 main.mm -o vt-llrc-probe \
  -framework Foundation -framework VideoToolbox -framework CoreMedia -framework CoreVideo
```

Run: `./vt-llrc-probe` (about 8 s; prints `arch: x86_64 (Rosetta on Apple Silicon)` first).
Building the same file with `-arch arm64` and running natively shows all four configs healthy.

## Steps to reproduce

1. On an Apple Silicon Mac, build the probe for x86_64 (command above) and run it under Rosetta.
2. Observe the four config sections. Each creates a `VTCompressionSession`
   (`EnableHardwareAcceleratedVideoEncoder=YES`, H.264 High profile, RealTime, no frame
   reordering, BT.709 primaries/transfer/matrix set on both the session and every pixel buffer),
   encodes 48 frames of a strong-chroma pattern (orange/blue halves + moving magenta bar + noise),
   then decodes the result and scans the output planes.

## Expected

All four configs decode with healthy chroma variance (the source pattern has near-full-range
Cb/Cr), as they do in a native arm64 process.

## Actual (x86_64 under Rosetta)

Output format is per-plane min/max/mean over all decoded frames (representative shape; exact
means vary with the noise seed):

```
=== classic RC + BGRA input (control) ===
  Y : min=<16..> max=<..235> mean=<mid>
  Cb: min=<low>  max=<high>  mean=<~128>   <- wide range: healthy
  Cr: min=<low>  max=<high>  mean=<~128>
  VERDICT: chroma healthy

=== LOW-LATENCY RC + BGRA input ===
  Y : min=<16..> max=<..235> mean=<mid>    <- luma correct
  Cb: min=  0 max=  3 mean=   0.0          <- chroma planes essentially zero
  Cr: min=  0 max=  3 mean=   0.0             (max <= 3 is quantization residue)
  VERDICT: CHROMA DEAD (green bug)

=== classic RC + NV12 input ===
  VERDICT: chroma healthy

=== LOW-LATENCY RC + NV12 input ===
  VERDICT: chroma healthy          <- same encoder, same settings; only input format differs
```

Only the {low-latency RC, BGRA} cell fails, and only under Rosetta. Since NV12 input through the
identical session is correct, the fault is isolated to the low-latency encoder's internal RGB→YCbCr
conversion in the translated environment. The failure survives explicit BT.709 attachments on input
buffers and session properties, full-range decode, and multi-slice access-unit assembly (ruled out
in the production integration before the probe was written); the emitted SPS is normal High-profile
4:2:0.

## Additional observations from the same environment

- `kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality` is rejected with `-12900`
  (`kVTPropertyNotSupportedErr`) by the low-latency encoder under Rosetta (probe output).
  `kVTCompressionPropertyKey_MaxFrameDelayCount` is likewise rejected with `-12900` by
  low-latency sessions (observed in our production encoder's property-status logs).
- Neither classic nor low-latency mode enforces `AverageBitRate`/`DataRateLimits` on
  low-compressibility content in this environment: the probe measures 77–122 Mbps against a
  42 Mbps target on noise-heavy frames.

## Impact

Real-time streaming from translated x86_64 processes (Wine/CrossOver-hosted VR and game streaming)
cannot use the low-latency encoder with RGB sources. Our workaround is a client-side RGB→NV12 Metal
compute pass before submission — with it, low-latency RC works well under Rosetta (encode p50 33 ms
→ 10.3 ms in our VR streaming pipeline), which shows the encoder itself is healthy apart from its
RGB input path.

