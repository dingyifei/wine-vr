# Feedback Assistant report — VideoToolbox ConstantBitRate accepted, then output callbacks cease, under Rosetta 2

**Title:** VideoToolbox HW H.264 encoder accepts kVTCompressionPropertyKey_ConstantBitRate but
output callbacks permanently cease after a few frames in x86_64 (Rosetta 2) processes on Apple
Silicon

**Area:** VideoToolbox
**Reproducibility:** Always
**macOS version:** macOS 26.5.2 (build 25F84)
**Hardware:** Apple Silicon Mac (observed on M3 Max); calling process running as x86_64 under Rosetta 2

## Summary

In an x86_64 process translated by Rosetta 2, setting `kVTCompressionPropertyKey_ConstantBitRate`
on a hardware H.264 `VTCompressionSession` is **accepted** (`VTSessionSetProperty` returns
`noErr`), but output callbacks then **cease after the first few frames**. The compression pipeline
stalls permanently: `VTCompressionSessionEncodeFrame` keeps returning `noErr` for subsequent
frames, the output callback is never invoked for them, `VTCompressionSessionCompleteFrames` does
not flush them, and no error is reported through any channel. Submitted pixel buffers are retained
forever, so a pool-based producer leaks every slot and encoding halts.

A silently-accepted property that permanently stalls the pipeline is significantly harder to
diagnose than a rejection; if CBR is unsupported for this encoder under translation, the
`VTSessionSetProperty` call should fail (as other unsupported properties do with `-12900`).

## Repro tool

Self-contained probe (single ObjC++ file, no dependencies): `tools/vt-llrc-probe/main.mm` from the
wine-vr project (file can be attached directly to this report). It encodes 48 synthetic 2496×1312
frames at 72 fps through the HW H.264 encoder and prints per-config callback counts and encode
statistics.

Build (must be x86_64 so it runs under Rosetta on Apple Silicon):

```
clang++ -arch x86_64 -std=c++17 -fobjc-arc -O2 main.mm -o vt-llrc-probe \
  -framework Foundation -framework VideoToolbox -framework CoreMedia -framework CoreVideo
```

## Steps to reproduce

1. On an Apple Silicon Mac, build the probe for x86_64 (command above).
2. Add one line to the probe's `RunConfig`, next to the existing `AverageBitRate` set:

   ```objc
   int cbr = kBitrate;
   CFNumberRef cbrRef = CFNumberCreate(nullptr, kCFNumberIntType, &cbr);
   setProp(kVTCompressionPropertyKey_ConstantBitRate, cbrRef);  // returns noErr
   CFRelease(cbrRef);
   ```

3. Run under Rosetta. The session is a standard HW H.264 `VTCompressionSession`
   (`EnableHardwareAcceleratedVideoEncoder=YES`, H.264 High profile, RealTime, no frame
   reordering); the stall reproduces with or without low-latency rate control.
4. Submit frames at a steady 72 fps cadence with `VTCompressionSessionEncodeFrame`.

## Expected

Either the property set is rejected (as unsupported properties are, e.g. `-12900`
`kVTPropertyNotSupportedErr`), or the session honors it and continues delivering one output
callback per submitted frame.

## Actual

The first few frames produce output callbacks, then callbacks stop entirely with no error
surfaced. The probe reports far fewer callbacks than submitted frames, e.g.:

```
  encoded: callbacks=34 frames=34 ...   <- 48 frames were submitted; 14 never produced output
```

In a real streaming pipeline the effect is total: in-flight pixel buffers are never released,
every encoder pool slot leaks within a fraction of a second, and video output halts permanently
(observed live in our VR streaming application before the property was removed — the encoder went
from a brief burst of output to 100% dropped frames with zero callbacks).

## Impact

Any translated x86_64 application attempting true constant-bitrate encoding (game/VR streaming
under Wine/CrossOver) hits an undiagnosable pipeline freeze instead of a property rejection. The
workaround is to avoid `ConstantBitRate` entirely and approximate CBR with
`AverageBitRate` + `DataRateLimits`.

*Related report filed separately: the low-latency encoder
(`EnableLowLatencyRateControl`) emits all-zero chroma for BGRA input in the same environment.*
