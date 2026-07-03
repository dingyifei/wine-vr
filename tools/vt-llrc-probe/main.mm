// VT low-latency rate-control probe.
//
// Encodes synthetic high-chroma frames through VideoToolbox H.264 HW under a
// matrix of {low-latency RC on/off} x {BGRA input / NV12 input}, decodes the
// bitstream back in-process, and reports per-plane chroma statistics.
// Built x86_64 so it reproduces the Rosetta translation environment of the
// wine-hosted encoder. Diagnoses the "all-zero chroma -> green" LL-RC bug and
// tests whether pre-converted NV12 input bypasses it.

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
    int64_t totalBytes = 0;
    // Encode latency: submit time per frame index (pts.value), delta measured
    // in the output callback. This is the stage where low-latency RC differs.
    std::vector<std::chrono::steady_clock::time_point> submitTimes;
    std::vector<double> latenciesMs;
};

static void EncodeOutput(void* refCon, void*, OSStatus status, VTEncodeInfoFlags,
                         CMSampleBufferRef sampleBuffer)
{
    auto* stream = (EncodedStream*)refCon;
    if (status != noErr || sampleBuffer == nullptr || !CMSampleBufferDataIsReady(sampleBuffer))
    {
        return;
    }
    stream->callbackCount++;
    CMTime cbPts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer);
    if (cbPts.value >= 0 && (size_t)cbPts.value < stream->submitTimes.size())
    {
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

static bool RunConfig(bool lowLatency, bool nv12Input, const char* label)
{
    printf("\n=== %s ===\n", label);

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
    // Chromium: DataRateLimits is incompatible with EnableLowLatencyRateControl
    // (undocumented). Only set it in classic mode.
    if (!lowLatency)
    {
        double peak = kBitrate / 8.0;
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

    for (int i = 0; i < kFrames; i++)
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
        VTCompressionSessionEncodeFrame(session, pb, pts, kCMTimeInvalid, nullptr, nullptr, nullptr);
        CVPixelBufferRelease(pb);
        // Pace at the real frame cadence so rate control and pipelining behave
        // like the live 72Hz encoder rather than a burst benchmark.
        std::this_thread::sleep_for(std::chrono::microseconds(1000000 / kFps));
    }
    VTCompressionSessionCompleteFrames(session, kCMTimeInvalid);
    VTCompressionSessionInvalidate(session);
    CFRelease(session);

    printf("  encoded: callbacks=%d frames=%zu avgKB/frame=%.0f (%.1f Mbps @%dfps)\n",
           stream.callbackCount, stream.annexb.size(),
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

int main()
{
    @autoreleasepool
    {
#if defined(__x86_64__)
        printf("arch: x86_64 (Rosetta on Apple Silicon)\n");
#else
        printf("arch: arm64 (NATIVE — this does NOT reproduce the wine environment!)\n");
#endif
        RunConfig(false, false, "classic RC + BGRA input (control)");
        RunConfig(true, false, "LOW-LATENCY RC + BGRA input (expect green bug)");
        RunConfig(false, true, "classic RC + NV12 input");
        RunConfig(true, true, "LOW-LATENCY RC + NV12 input (the hypothesis)");
        printf("\ndone\n");
    }
    return 0;
}
