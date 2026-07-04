// VT low-latency rate-control probe.
//
// Default mode: encodes synthetic high-chroma frames through VideoToolbox
// H.264 HW under a matrix of {low-latency RC on/off} x {BGRA input / NV12
// input}, decodes the bitstream back in-process, and reports per-plane chroma
// statistics. Diagnoses the "all-zero chroma -> green" LL-RC bug and tests
// whether pre-converted NV12 input bypasses it.
//
// --cbr mode: additionally sets kVTCompressionPropertyKey_ConstantBitRate.
// Under Rosetta the property is accepted (noErr) but output callbacks cease
// after the first few frames; the probe reports submitted-vs-callback counts.
//
// Built x86_64 so it reproduces the Rosetta translation environment of the
// wine-hosted encoder.

#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <VideoToolbox/VideoToolbox.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <thread>
#include <vector>

static const int kWidth = 2496;
static const int kHeight = 1312;
static const int kFps = 72;
static const int kFrames = 48;
static const int kBitrate = 42 * 1000 * 1000;

struct EncodedStream
{
    std::vector<std::vector<uint8_t>> annexb; // one entry per frame (VCL + prefixed params on IDR)
    std::vector<uint8_t> avccExtradata;       // from format description (SPS/PPS)
    CMFormatDescriptionRef formatDesc = nullptr;
    int callbackCount = 0;
    int droppedCount = 0;   // callback fired with kVTEncodeInfo_FrameDropped / no sample
    int errorCount = 0;     // callback fired with status != noErr
    std::vector<int64_t> outputPts;  // pts of frames that produced real output
    std::vector<int64_t> droppedPts; // pts of frames the encoder reported dropped
    int64_t totalBytes = 0;
    // Encode latency: submit time per frame index (pts.value), delta measured
    // in the output callback. This is the stage where low-latency RC differs.
    std::vector<std::chrono::steady_clock::time_point> submitTimes;
    std::vector<double> latenciesMs;
};

static void EncodeOutput(void* refCon, void* frameRefCon, OSStatus status, VTEncodeInfoFlags infoFlags,
                         CMSampleBufferRef sampleBuffer)
{
    auto* stream = (EncodedStream*)refCon;
    if (status != noErr)
    {
        stream->errorCount++;
        printf("    [cb] frame %ld status=%d\n", (long)(intptr_t)frameRefCon, (int)status);
        return;
    }
    if ((infoFlags & kVTEncodeInfo_FrameDropped) != 0 || sampleBuffer == nullptr ||
        !CMSampleBufferDataIsReady(sampleBuffer))
    {
        // A drop still releases the pixel buffer; a true stall never calls back.
        stream->droppedCount++;
        stream->droppedPts.push_back((int64_t)(intptr_t)frameRefCon);
        return;
    }
    stream->callbackCount++;
    CMTime cbPts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
    if (cbPts.value >= 0 && (size_t)cbPts.value < stream->submitTimes.size())
    {
        stream->outputPts.push_back(cbPts.value);
        stream->latenciesMs.push_back(
            std::chrono::duration<double, std::milli>(
                std::chrono::steady_clock::now() - stream->submitTimes[(size_t)cbPts.value])
                .count());
    }
    CMFormatDescriptionRef desc = CMSampleBufferGetFormatDescription(sampleBuffer);
    if (desc != nullptr && stream->formatDesc == nullptr)
    {
        stream->formatDesc = (CMFormatDescriptionRef)CFRetain(desc);
    }
    CMBlockBufferRef block = CMSampleBufferGetDataBuffer(sampleBuffer);
    size_t length = 0;
    char* data = nullptr;
    if (CMBlockBufferGetDataPointer(block, 0, nullptr, &length, &data) == kCMBlockBufferNoErr)
    {
        stream->annexb.emplace_back((uint8_t*)data, (uint8_t*)data + length);
        stream->totalBytes += (int64_t)length;
    }
}

// Per-pixel hash noise mixed over a strong-chroma base pattern. High entropy
// forces the encoder against its bitrate budget (like real game content) so
// rate-control latency behavior is exercised, not just pipeline latency.
static inline uint32_t Hash32(uint32_t v)
{
    v ^= v >> 16;
    v *= 0x7feb352d;
    v ^= v >> 15;
    v *= 0x846ca68b;
    v ^= v >> 16;
    return v;
}

static inline void PatternRgb(int x, int y, int frameIndex, uint8_t& r, uint8_t& g, uint8_t& b)
{
    int barX = (frameIndex * 37) % kWidth;
    if (x >= barX && x < barX + 120) { b = 255; g = 0; r = 255; }       // magenta bar
    else if (x < kWidth / 2)         { b = 0;   g = 128; r = 255; }     // orange
    else                             { b = 255; g = 64;  r = 0; }       // blue
    uint32_t h = Hash32((uint32_t)(y * kWidth + x) * 2654435761u + (uint32_t)frameIndex * 97);
    r = (uint8_t)std::clamp((int)r + (int)(h & 0x7F) - 64, 0, 255);
    g = (uint8_t)std::clamp((int)g + (int)((h >> 8) & 0x7F) - 64, 0, 255);
    b = (uint8_t)std::clamp((int)b + (int)((h >> 16) & 0x7F) - 64, 0, 255);
}

static void FillBGRA(CVPixelBufferRef pb, int frameIndex)
{
    CVPixelBufferLockBaseAddress(pb, 0);
    uint8_t* base = (uint8_t*)CVPixelBufferGetBaseAddress(pb);
    size_t stride = CVPixelBufferGetBytesPerRow(pb);
    for (int y = 0; y < kHeight; y++)
    {
        uint8_t* row = base + y * stride;
        for (int x = 0; x < kWidth; x++)
        {
            uint8_t r, g, b;
            PatternRgb(x, y, frameIndex, r, g, b);
            row[x * 4 + 0] = b;
            row[x * 4 + 1] = g;
            row[x * 4 + 2] = r;
            row[x * 4 + 3] = 255;
        }
    }
    CVPixelBufferUnlockBaseAddress(pb, 0);
}

// BT.709 video-range RGB->YCbCr for the NV12 fill (same content as FillBGRA).
static inline void Rgb709(uint8_t r, uint8_t g, uint8_t b, uint8_t& Y, uint8_t& Cb, uint8_t& Cr)
{
    float rf = r / 255.0f, gf = g / 255.0f, bf = b / 255.0f;
    float y = 0.2126f * rf + 0.7152f * gf + 0.0722f * bf;
    float cb = (bf - y) / 1.8556f;
    float cr = (rf - y) / 1.5748f;
    Y = (uint8_t)std::clamp(16.0f + 219.0f * y, 0.0f, 255.0f);
    Cb = (uint8_t)std::clamp(128.0f + 224.0f * cb, 0.0f, 255.0f);
    Cr = (uint8_t)std::clamp(128.0f + 224.0f * cr, 0.0f, 255.0f);
}

static void FillNV12(CVPixelBufferRef pb, int frameIndex)
{
    CVPixelBufferLockBaseAddress(pb, 0);
    uint8_t* yBase = (uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pb, 0);
    uint8_t* cBase = (uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pb, 1);
    size_t yStride = CVPixelBufferGetBytesPerRowOfPlane(pb, 0);
    size_t cStride = CVPixelBufferGetBytesPerRowOfPlane(pb, 1);
    for (int y = 0; y < kHeight; y++)
    {
        uint8_t* row = yBase + y * yStride;
        for (int x = 0; x < kWidth; x++)
        {
            uint8_t r, g, b, Y, Cb, Cr;
            PatternRgb(x, y, frameIndex, r, g, b);
            Rgb709(r, g, b, Y, Cb, Cr);
            row[x] = Y;
        }
    }
    for (int y = 0; y < kHeight / 2; y++)
    {
        uint8_t* row = cBase + y * cStride;
        for (int x = 0; x < kWidth / 2; x++)
        {
            uint8_t r, g, b, Y, Cb, Cr;
            PatternRgb(x * 2, y * 2, frameIndex, r, g, b);
            Rgb709(r, g, b, Y, Cb, Cr);
            row[x * 2 + 0] = Cb;
            row[x * 2 + 1] = Cr;
        }
    }
    CVPixelBufferUnlockBaseAddress(pb, 0);
}

struct PlaneStats
{
    int minv = 255, maxv = 0;
    double mean = 0.0;
};

struct DecodeStats
{
    PlaneStats y, cb, cr;
    int framesDecoded = 0;
};

static void AccumulatePlane(const uint8_t* base, size_t stride, int w, int h, int step,
                            PlaneStats& s, double& sum, int64_t& count)
{
    for (int yy = 0; yy < h; yy++)
    {
        const uint8_t* row = base + yy * stride;
        for (int xx = 0; xx < w; xx += step)
        {
            int v = row[xx];
            s.minv = std::min(s.minv, v);
            s.maxv = std::max(s.maxv, v);
            sum += v;
            count++;
        }
    }
}

static void DecodeOutput(void* refCon, void*, OSStatus status, VTDecodeInfoFlags,
                         CVImageBufferRef imageBuffer, CMTime, CMTime)
{
    auto* stats = (DecodeStats*)refCon;
    if (status != noErr || imageBuffer == nullptr)
    {
        return;
    }
    CVPixelBufferRef pb = (CVPixelBufferRef)imageBuffer;
    CVPixelBufferLockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
    const uint8_t* yBase = (const uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pb, 0);
    const uint8_t* cBase = (const uint8_t*)CVPixelBufferGetBaseAddressOfPlane(pb, 1);
    size_t yStride = CVPixelBufferGetBytesPerRowOfPlane(pb, 0);
    size_t cStride = CVPixelBufferGetBytesPerRowOfPlane(pb, 1);
    int w = (int)CVPixelBufferGetWidthOfPlane(pb, 0);
    int h = (int)CVPixelBufferGetHeightOfPlane(pb, 0);
    int cw = (int)CVPixelBufferGetWidthOfPlane(pb, 1);
    int ch = (int)CVPixelBufferGetHeightOfPlane(pb, 1);

    static double ySum, cbSum, crSum;
    static int64_t yCount, cbCount, crCount;
    if (stats->framesDecoded == 0)
    {
        ySum = cbSum = crSum = 0.0;
        yCount = cbCount = crCount = 0;
    }
    AccumulatePlane(yBase, yStride, w, h, 7, stats->y, ySum, yCount);
    // CbCr interleaved: even offsets = Cb, odd = Cr.
    for (int yy = 0; yy < ch; yy++)
    {
        const uint8_t* row = cBase + yy * cStride;
        for (int xx = 0; xx < cw * 2; xx += 8)
        {
            int cb = row[xx], cr = row[xx + 1];
            stats->cb.minv = std::min(stats->cb.minv, cb);
            stats->cb.maxv = std::max(stats->cb.maxv, cb);
            stats->cr.minv = std::min(stats->cr.minv, cr);
            stats->cr.maxv = std::max(stats->cr.maxv, cr);
            cbSum += cb; crSum += cr; cbCount++; crCount++;
        }
    }
    stats->framesDecoded++;
    stats->y.mean = yCount ? ySum / yCount : 0;
    stats->cb.mean = cbCount ? cbSum / cbCount : 0;
    stats->cr.mean = crCount ? crSum / crCount : 0;
    CVPixelBufferUnlockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
}

// stallMode: 0 = chroma matrix (decode + plane stats),
//            1 = callback accounting only (control for --cbr),
//            2 = callback accounting + set kVTCompressionPropertyKey_ConstantBitRate,
//            3 = mode 2 plus the full oxrsys production property set at the time
//                the live stall was observed (CABAC, keyframe interval,
//                MaxFrameDelayCount=0, 1.5x DataRateLimits headroom).
static bool RunConfig(bool lowLatency, bool nv12Input, const char* label, int stallMode = 0)
{
    printf("\n=== %s ===\n", label);
    const int frameCount = (stallMode != 0) ? kFrames * 5 : kFrames;

    EncodedStream stream;
    NSMutableDictionary* encoderSpec = [@{
        (NSString*)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: @YES,
    } mutableCopy];
    if (lowLatency)
    {
        encoderSpec[(NSString*)kVTVideoEncoderSpecification_EnableLowLatencyRateControl] = @YES;
    }

    VTCompressionSessionRef session = nullptr;
    OSStatus status = VTCompressionSessionCreate(
        kCFAllocatorDefault, kWidth, kHeight, kCMVideoCodecType_H264,
        (__bridge CFDictionaryRef)encoderSpec, nullptr, kCFAllocatorDefault,
        EncodeOutput, &stream, &session);
    if (status != noErr)
    {
        printf("  SESSION CREATE FAILED: %d\n", (int)status);
        return false;
    }

    auto setProp = [&](CFStringRef key, CFTypeRef value)
    {
        OSStatus s = VTSessionSetProperty(session, key, value);
        if (s != noErr)
        {
            printf("  prop %s REJECTED (%d)\n",
                   [(__bridge NSString*)key UTF8String], (int)s);
        }
    };
    setProp(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    setProp(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    setProp(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
    int br = kBitrate;
    CFNumberRef brRef = CFNumberCreate(nullptr, kCFNumberIntType, &br);
    setProp(kVTCompressionPropertyKey_AverageBitRate, brRef);
    CFRelease(brRef);
    if (stallMode == 3)
    {
        // Match the oxrsys VideoEncoder property set from the 2026-07-03 live
        // CBR experiment (classic RC, BGRA input, balanced preset).
        setProp(kVTCompressionPropertyKey_H264EntropyMode, kVTH264EntropyMode_CABAC);
        int keyframeInterval = 10 * kFps;
        CFNumberRef kfRef = CFNumberCreate(nullptr, kCFNumberIntType, &keyframeInterval);
        setProp(kVTCompressionPropertyKey_MaxKeyFrameInterval, kfRef);
        CFRelease(kfRef);
        double keyframeDuration = 10.0;
        CFNumberRef kfdRef = CFNumberCreate(nullptr, kCFNumberFloat64Type, &keyframeDuration);
        setProp(kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, kfdRef);
        CFRelease(kfdRef);
        int maxFrameDelay = 0;
        CFNumberRef delayRef = CFNumberCreate(nullptr, kCFNumberIntType, &maxFrameDelay);
        setProp(kVTCompressionPropertyKey_MaxFrameDelayCount, delayRef);
        CFRelease(delayRef);
    }
    if (stallMode >= 2)
    {
        // Print the status unconditionally: the bug is that this set is
        // ACCEPTED (noErr) under Rosetta yet the pipeline then stalls.
        if (@available(macOS 13.0, *))
        {
            int cbrBits = kBitrate;
            CFNumberRef cbrRef = CFNumberCreate(nullptr, kCFNumberIntType, &cbrBits);
            OSStatus s = VTSessionSetProperty(session, kVTCompressionPropertyKey_ConstantBitRate, cbrRef);
            CFRelease(cbrRef);
            printf("  ConstantBitRate=%d -> VTSessionSetProperty status=%d (%s)\n",
                   kBitrate, (int)s, s == noErr ? "accepted" : "rejected");
        }
        else
        {
            printf("  ConstantBitRate unavailable before macOS 13 — skipping config\n");
            VTCompressionSessionInvalidate(session);
            CFRelease(session);
            return false;
        }
    }
    // Chromium: DataRateLimits is incompatible with EnableLowLatencyRateControl
    // (undocumented). Only set it in classic mode.
    if (!lowLatency)
    {
        // Production used 1.5x headroom at the time of the CBR experiment.
        double peak = kBitrate * (stallMode == 3 ? 1.5 : 1.0) / 8.0;
        setProp(kVTCompressionPropertyKey_DataRateLimits,
                (__bridge CFArrayRef)@[ @(peak), @(1.0) ]);
    }
    setProp(kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality, kCFBooleanTrue);
    int fps = kFps;
    CFNumberRef fpsRef = CFNumberCreate(nullptr, kCFNumberIntType, &fps);
    setProp(kVTCompressionPropertyKey_ExpectedFrameRate, fpsRef);
    CFRelease(fpsRef);
    setProp(kVTCompressionPropertyKey_ColorPrimaries, kCVImageBufferColorPrimaries_ITU_R_709_2);
    setProp(kVTCompressionPropertyKey_TransferFunction, kCVImageBufferTransferFunction_ITU_R_709_2);
    setProp(kVTCompressionPropertyKey_YCbCrMatrix, kCVImageBufferYCbCrMatrix_ITU_R_709_2);
    VTCompressionSessionPrepareToEncodeFrames(session);

    CFBooleanRef usingHw = nullptr;
    if (VTSessionCopyProperty(session, kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                              kCFAllocatorDefault, &usingHw) == noErr && usingHw)
    {
        printf("  hardware=%s\n", CFBooleanGetValue(usingHw) ? "yes" : "NO (software!)");
        CFRelease(usingHw);
    }

    OSType pixelFormat = nv12Input ? kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                                   : kCVPixelFormatType_32BGRA;
    NSDictionary* pbAttrs = @{
        (NSString*)kCVPixelBufferPixelFormatTypeKey: @(pixelFormat),
        (NSString*)kCVPixelBufferIOSurfacePropertiesKey: @{},
    };

    for (int i = 0; i < frameCount; i++)
    {
        CVPixelBufferRef pb = nullptr;
        if (CVPixelBufferCreate(nullptr, kWidth, kHeight, pixelFormat,
                                (__bridge CFDictionaryRef)pbAttrs, &pb) != kCVReturnSuccess)
        {
            printf("  pixel buffer create failed at frame %d\n", i);
            break;
        }
        if (nv12Input) { FillNV12(pb, i); } else { FillBGRA(pb, i); }
        CVBufferSetAttachment(pb, kCVImageBufferColorPrimariesKey,
                              kCVImageBufferColorPrimaries_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);
        CVBufferSetAttachment(pb, kCVImageBufferTransferFunctionKey,
                              kCVImageBufferTransferFunction_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);
        CVBufferSetAttachment(pb, kCVImageBufferYCbCrMatrixKey,
                              kCVImageBufferYCbCrMatrix_ITU_R_709_2, kCVAttachmentMode_ShouldPropagate);

        CMTime pts = CMTimeMake(i, kFps);
        stream.submitTimes.push_back(std::chrono::steady_clock::now());
        VTCompressionSessionEncodeFrame(session, pb, pts, kCMTimeInvalid, nullptr,
                                        (void*)(intptr_t)i, nullptr);
        CVPixelBufferRelease(pb);
        // Pace at the real frame cadence so rate control and pipelining behave
        // like the live 72Hz encoder rather than a burst benchmark.
        std::this_thread::sleep_for(std::chrono::microseconds(1000000 / kFps));
    }
    VTCompressionSessionCompleteFrames(session, kCMTimeInvalid);
    VTCompressionSessionInvalidate(session);
    CFRelease(session);

    printf("  encoded: callbacks=%d dropped=%d errors=%d frames=%zu avgKB/frame=%.0f (%.1f Mbps @%dfps)\n",
           stream.callbackCount, stream.droppedCount, stream.errorCount, stream.annexb.size(),
           stream.annexb.empty() ? 0.0 : stream.totalBytes / 1024.0 / stream.annexb.size(),
           stream.annexb.empty() ? 0.0
                                 : stream.totalBytes * 8.0 * kFps / stream.annexb.size() / 1e6,
           kFps);
    if (!stream.latenciesMs.empty())
    {
        auto lats = stream.latenciesMs;
        std::sort(lats.begin(), lats.end());
        printf("  encode latency (submit->callback): p50=%.1fms p95=%.1fms max=%.1fms (n=%zu)\n",
               lats[lats.size() / 2], lats[(size_t)(lats.size() * 0.95)], lats.back(), lats.size());
    }
    if (stallMode != 0)
    {
        // Chroma decode is irrelevant here; the CBR bug is missing callbacks.
        // Distinguish frames the encoder REPORTED dropped (callback fires,
        // pixel buffer released) from frames that got NO callback at all
        // (buffer retained forever — the true stall symptom).
        int submitted = (int)stream.submitTimes.size();
        int accounted = stream.callbackCount + stream.droppedCount + stream.errorCount;
        int silent = submitted - accounted;
        if (!stream.droppedPts.empty())
        {
            printf("  reported-dropped pts:");
            for (int64_t p : stream.droppedPts) printf(" %lld", (long long)p);
            printf("\n");
        }
        if (silent > 0)
        {
            std::vector<bool> got(submitted, false);
            for (int64_t p : stream.outputPts) if (p >= 0 && p < submitted) got[(size_t)p] = true;
            for (int64_t p : stream.droppedPts) if (p >= 0 && p < submitted) got[(size_t)p] = true;
            printf("  silent (no callback ever) pts:");
            for (int i2 = 0; i2 < submitted; i2++) if (!got[(size_t)i2]) printf(" %d", i2);
            printf("\n");
            printf("  VERDICT: PIPELINE STALL — %d of %d submitted frames never produced any callback\n",
                   silent, submitted);
        }
        else
        {
            printf("  VERDICT: no stall — every submitted frame produced a callback (%d output, %d dropped)\n",
                   stream.callbackCount, stream.droppedCount);
        }
        if (stream.formatDesc != nullptr)
        {
            CFRelease(stream.formatDesc);
        }
        return silent == 0;
    }
    if (stream.formatDesc == nullptr || stream.annexb.empty())
    {
        printf("  NO OUTPUT — encoder stalled or produced nothing\n");
        return false;
    }

    // Decode back and measure planes.
    DecodeStats dstats;
    VTDecompressionOutputCallbackRecord cb = { DecodeOutput, &dstats };
    NSDictionary* outAttrs = @{
        (NSString*)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
    };
    VTDecompressionSessionRef dec = nullptr;
    status = VTDecompressionSessionCreate(nullptr, stream.formatDesc, nullptr,
                                          (__bridge CFDictionaryRef)outAttrs, &cb, &dec);
    if (status != noErr)
    {
        printf("  DECODER CREATE FAILED: %d\n", (int)status);
        CFRelease(stream.formatDesc);
        return false;
    }
    for (size_t i = 0; i < stream.annexb.size(); i++)
    {
        auto& frame = stream.annexb[i];
        CMBlockBufferRef block = nullptr;
        CMBlockBufferCreateWithMemoryBlock(nullptr, frame.data(), frame.size(), kCFAllocatorNull,
                                           nullptr, 0, frame.size(), 0, &block);
        CMSampleBufferRef sample = nullptr;
        size_t sizes[1] = { frame.size() };
        CMSampleBufferCreateReady(nullptr, block, stream.formatDesc, 1, 0, nullptr, 1, sizes, &sample);
        if (sample != nullptr)
        {
            VTDecompressionSessionDecodeFrame(dec, sample, 0, nullptr, nullptr);
            CFRelease(sample);
        }
        if (block != nullptr)
        {
            CFRelease(block);
        }
    }
    VTDecompressionSessionWaitForAsynchronousFrames(dec);
    VTDecompressionSessionInvalidate(dec);
    CFRelease(dec);
    CFRelease(stream.formatDesc);

    printf("  decoded %d frames\n", dstats.framesDecoded);
    printf("  Y : min=%3d max=%3d mean=%6.1f\n", dstats.y.minv, dstats.y.maxv, dstats.y.mean);
    printf("  Cb: min=%3d max=%3d mean=%6.1f\n", dstats.cb.minv, dstats.cb.maxv, dstats.cb.mean);
    printf("  Cr: min=%3d max=%3d mean=%6.1f\n", dstats.cr.minv, dstats.cr.maxv, dstats.cr.mean);
    const bool chromaDead = dstats.cb.maxv <= 20 && dstats.cr.maxv <= 20;
    const bool chromaHealthy = (dstats.cb.maxv - dstats.cb.minv) > 60 &&
                               (dstats.cr.maxv - dstats.cr.minv) > 60;
    printf("  VERDICT: %s\n", chromaDead      ? "CHROMA DEAD (green bug)"
                              : chromaHealthy ? "chroma healthy"
                                              : "chroma SUSPICIOUS (low variance)");
    return true;
}

int main(int argc, char** argv)
{
    bool cbrMode = false;
    for (int i = 1; i < argc; i++)
    {
        if (strcmp(argv[i], "--cbr") == 0)
        {
            cbrMode = true;
        }
        else
        {
            fprintf(stderr,
                    "usage: %s [--cbr]\n"
                    "  (default)  chroma matrix: {LL-RC on/off} x {BGRA/NV12 input}\n"
                    "  --cbr      ConstantBitRate stall repro (accepted but callbacks cease)\n",
                    argv[0]);
            return 2;
        }
    }
    @autoreleasepool
    {
#if defined(__x86_64__)
        printf("arch: x86_64 (Rosetta on Apple Silicon)\n");
#else
        printf("arch: arm64 (NATIVE — this does NOT reproduce the wine environment!)\n");
#endif
        if (cbrMode)
        {
            // NV12 input keeps the LL-RC BGRA chroma bug out of the picture,
            // so any callback deficit is attributable to ConstantBitRate.
            // Controls (stallMode 1) use identical accounting without the prop.
            RunConfig(false, true, "classic RC + NV12 (control, no CBR)", 1);
            RunConfig(false, true, "classic RC + NV12 + ConstantBitRate", 2);
            RunConfig(true, true, "LOW-LATENCY RC + NV12 (control, no CBR)", 1);
            RunConfig(true, true, "LOW-LATENCY RC + NV12 + ConstantBitRate", 2);
            RunConfig(false, false, "PRODUCTION MIRROR: classic RC + BGRA + CABAC + 1.5x limits + CBR", 3);
        }
        else
        {
            RunConfig(false, false, "classic RC + BGRA input (control)");
            RunConfig(true, false, "LOW-LATENCY RC + BGRA input (expect green bug)");
            RunConfig(false, true, "classic RC + NV12 input");
            RunConfig(true, true, "LOW-LATENCY RC + NV12 input (the hypothesis)");
        }
        printf("\ndone\n");
    }
    return 0;
}
