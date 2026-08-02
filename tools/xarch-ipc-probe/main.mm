// Cross-arch IPC probe — Gates B and B2 of the native-arm64 out-of-process
// encoder plan.
//
// Gate B (default): an x86_64 parent (simulating the Rosetta/Wine runtime)
// posix_spawns this same tool's arm64 build as a child (the future encoder
// helper), rendezvouses over Mach (preferred: parent installs a receive right
// as the child's TASK_BOOTSTRAP_PORT via posix_spawnattr_setspecialport_np;
// fallback: bootstrap_register/bootstrap_look_up under a per-spawn name),
// transfers 3 IOSurface-backed 32BGRA 3008x1664 CVPixelBuffer surfaces by
// send right (IOSurfaceCreateMachPort -> mach_msg port descriptor ->
// IOSurfaceLookupFromMachPort), then stress-rotates them: the parent
// GPU-writes a per-frame deterministic pattern (Metal compute; frame index
// encoded in a pixel block AND mixed into every pixel's hash), submits
// {slot, frameIndex} over a socketpair AFTER the command buffer's completed
// handler fires, and the CHILD verifies the full surface with its own Metal
// compute pass (GPU read, no CPU lock) — zero tolerance for stale patterns.
// 100k rotations by default; round-trip p50/p95/p99 doubles as an early
// Gate C signal. Then a crash/leak audit: SIGKILL the child mid-stream,
// respawn, re-register the SAME surfaces, continue — 5 cycles, watching the
// parent's mach_port_names count for monotonic growth.
//
// NOTE the child restores its real bootstrap port (sent by the parent in the
// first Mach message) before touching Metal/VT — with the parent's port still
// installed as TASK_BOOTSTRAP_PORT, XPC service lookups (MTLCompilerService,
// VideoToolbox) would fail. This is a design requirement for the production
// helper if it uses the special-port rendezvous.
//
// Gate B2 (--b2): the foreign-surface VT seam. The arm64 child wraps each
// received IOSurface ONCE with CVPixelBufferCreateWithIOSurface (+ BT.709
// ShouldPropagate attachments) and encodes hardware-REQUIRED HEVC Main with
// low-latency rate control (property set copied from vt-llrc-probe's Gate A
// --gpu config). The parent GPU-writes vt-llrc-probe's bars+noise pattern at
// 72fps cadence; all child bookkeeping writes happen BEFORE EncodeFrame
// (LL-RC callbacks can fire before it returns). At the end the child decodes
// the stream back in-process and verifies chroma; with --matrix the frames
// are flat BT.709 bands and decoded band means must match the limited-range
// reference within tolerance 8. GPU-done -> child-VT-callback p50/p95/p99 is
// reported from the child.
//
// Build (parent MUST be x86_64, child MUST be arm64):
//   clang++ -arch x86_64 -std=c++17 -fobjc-arc -O2 main.mm -o xarch-ipc-probe \
//     -framework Metal -framework CoreVideo -framework CoreMedia \
//     -framework VideoToolbox -framework IOSurface -framework Foundation
//   clang++ -arch arm64 ... -o xarch-ipc-probe-arm64   (same flags otherwise)
//
// Run:  ./xarch-ipc-probe [--rotations N]        Gate B
//       ./xarch-ipc-probe --b2 [--frames N]      Gate B2 (bars+noise content)
//       ./xarch-ipc-probe --b2 --matrix          Gate B2 BT.709 band check

#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#import <Foundation/Foundation.h>
#import <IOSurface/IOSurface.h>
#import <Metal/Metal.h>
#import <VideoToolbox/VideoToolbox.h>

#include <errno.h>
#include <mach/mach.h>
#include <mach/mach_time.h>
#include <servers/bootstrap.h>
#include <signal.h>
#include <spawn.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <string>
#include <thread>
#include <vector>

extern char** environ;

static int gWidth = 3008;
static int gHeight = 1664;
static const int kFps = 72;
static const int kBitrate = 42 * 1000 * 1000;
static const int kSlots = 3;
static bool gMatrixMode = false;

static mach_timebase_info_data_t gTimebase;
static double TicksToMs(uint64_t t)
{
    return (double)t * gTimebase.numer / gTimebase.denom / 1e6;
}

// Cross-arch gotcha: mach_absolute_time ticks are NOT comparable across the
// Rosetta boundary (x86_64 sees a 1/1 ns timebase, native arm64 125/3).
// Timestamps that cross the process boundary go over the wire in nanoseconds.
static uint64_t NowNs()
{
    return mach_absolute_time() * gTimebase.numer / gTimebase.denom;
}

static void PrintPercentiles(const char* label, std::vector<double>& v)
{
    if (v.empty())
    {
        printf("  %s: no samples\n", label);
        return;
    }
    std::sort(v.begin(), v.end());
    printf("  %s: p50=%.3fms p95=%.3fms p99=%.3fms max=%.3fms (n=%zu)\n", label,
           v[v.size() / 2], v[(size_t)(v.size() * 0.95)], v[(size_t)(v.size() * 0.99)],
           v.back(), v.size());
}

// ---------------------------------------------------------------------------
// Socket protocol (parent <-> child data plane).
// ---------------------------------------------------------------------------

enum : uint32_t
{
    kMsgSubmit = 1, // parent -> child: frame written, GPU done
    kMsgReply = 2,  // child -> parent: verified (B) / VT callback fired (B2)
    kMsgEnd = 3,    // parent -> child: stream over (B2: frameIndex = total)
    kMsgEndAck = 4, // child -> parent: final verdict in ok
    kMsgReady = 5,  // child -> parent: surfaces validated, session up
};

struct SockMsg
{
    uint32_t type = 0;
    uint32_t slot = 0;
    uint64_t frameIndex = 0;
    uint64_t tGpuDone = 0; // mach_absolute_time at parent's completed handler
    uint32_t ok = 0;
    uint32_t mismatches = 0;
};

static bool WriteAll(int fd, const void* buf, size_t len)
{
    const uint8_t* p = (const uint8_t*)buf;
    while (len > 0)
    {
        ssize_t n = write(fd, p, len);
        if (n < 0)
        {
            if (errno == EINTR)
            {
                continue;
            }
            return false;
        }
        p += n;
        len -= (size_t)n;
    }
    return true;
}

static bool ReadAll(int fd, void* buf, size_t len)
{
    uint8_t* p = (uint8_t*)buf;
    while (len > 0)
    {
        ssize_t n = read(fd, p, len);
        if (n < 0)
        {
            if (errno == EINTR)
            {
                continue;
            }
            return false;
        }
        if (n == 0)
        {
            return false; // EOF
        }
        p += n;
        len -= (size_t)n;
    }
    return true;
}

// ---------------------------------------------------------------------------
// Mach protocol (rendezvous + surface transfer).
// ---------------------------------------------------------------------------

enum : mach_msg_id_t
{
    kMachCheckin = 0x1001,   // child -> parent: a = child pid, port = child rx
    kMachBootstrap = 0x1002, // parent -> child: port = the REAL bootstrap port
    kMachSurface = 0x1003,   // parent -> child: a = slot, b = w, c = h
};

struct MachMsg
{
    mach_msg_header_t hdr;
    mach_msg_body_t body;
    mach_msg_port_descriptor_t port;
    uint32_t a, b, c, d;
};

struct MachMsgRecv
{
    MachMsg m;
    mach_msg_trailer_t trailer;
};

static kern_return_t SendPortMsg(mach_port_t dest, mach_msg_id_t id, mach_port_t port,
                                 mach_msg_type_name_t disposition, uint32_t a, uint32_t b,
                                 uint32_t c)
{
    MachMsg msg = {};
    msg.hdr.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
    msg.hdr.msgh_size = sizeof(msg);
    msg.hdr.msgh_remote_port = dest;
    msg.hdr.msgh_id = id;
    msg.body.msgh_descriptor_count = 1;
    msg.port.name = port;
    msg.port.disposition = disposition;
    msg.port.type = MACH_MSG_PORT_DESCRIPTOR;
    msg.a = a;
    msg.b = b;
    msg.c = c;
    return mach_msg(&msg.hdr, MACH_SEND_MSG, sizeof(msg), 0, MACH_PORT_NULL,
                    MACH_MSG_TIMEOUT_NONE, MACH_PORT_NULL);
}

static kern_return_t RecvPortMsg(mach_port_t rx, MachMsgRecv& out, mach_msg_timeout_t timeoutMs)
{
    memset(&out, 0, sizeof(out));
    return mach_msg(&out.m.hdr, MACH_RCV_MSG | (timeoutMs != 0 ? MACH_RCV_TIMEOUT : 0), 0,
                    sizeof(out), rx, timeoutMs, MACH_PORT_NULL);
}

static int CountPorts()
{
    mach_port_name_array_t names = nullptr;
    mach_port_type_array_t types = nullptr;
    mach_msg_type_number_t n = 0, tn = 0;
    if (mach_port_names(mach_task_self(), &names, &n, &types, &tn) != KERN_SUCCESS)
    {
        return -1;
    }
    vm_deallocate(mach_task_self(), (vm_address_t)names, n * sizeof(*names));
    vm_deallocate(mach_task_self(), (vm_address_t)types, tn * sizeof(*types));
    return (int)n;
}

// ---------------------------------------------------------------------------
// Metal kernels. write_pattern/verify_pattern share expected_color so the
// parent-written and child-expected patterns are identical by construction:
// pixels (0..3, 0) literally encode the frame index bytes; every other pixel
// is a hash of (pixel index, frameIndex). b2_pattern is vt-llrc-probe's
// bars+noise / BT.709 band content for the encoder gate.
// ---------------------------------------------------------------------------

struct GpuParams
{
    uint32_t width, height, frameIndex, matrixMode;
};

static const char* kMetalSource = R"MSL(
#include <metal_stdlib>
using namespace metal;

struct Params { uint width; uint height; uint frameIndex; uint matrixMode; };

static uint hash32(uint v)
{
    v ^= v >> 16; v *= 0x7feb352du; v ^= v >> 15; v *= 0x846ca68bu; v ^= v >> 16;
    return v;
}

static float4 expected_color(uint2 gid, constant Params& p)
{
    if (gid.y == 0u && gid.x < 4u)
    {
        float v = float((p.frameIndex >> (8u * gid.x)) & 0xFFu) / 255.0;
        return float4(v, v, v, 1.0);
    }
    uint h = hash32((gid.y * p.width + gid.x) * 2654435761u ^
                    (p.frameIndex * 0x9E3779B9u + 0x85EBCA6Bu));
    return float4(float(h & 0xFFu), float((h >> 8) & 0xFFu), float((h >> 16) & 0xFFu), 255.0) /
           255.0;
}

kernel void write_pattern(texture2d<float, access::write> dst [[texture(0)]],
                          constant Params& p [[buffer(0)]],
                          uint2 gid [[thread_position_in_grid]])
{
    if (gid.x >= p.width || gid.y >= p.height) { return; }
    dst.write(expected_color(gid, p), gid);
}

kernel void verify_pattern(texture2d<float, access::read> src [[texture(0)]],
                           constant Params& p [[buffer(0)]],
                           device atomic_uint* mismatches [[buffer(1)]],
                           uint2 gid [[thread_position_in_grid]])
{
    if (gid.x >= p.width || gid.y >= p.height) { return; }
    uint4 got = uint4(rint(src.read(gid) * 255.0));
    uint4 want = uint4(rint(expected_color(gid, p) * 255.0));
    if (any(got != want))
    {
        atomic_fetch_add_explicit(mismatches, 1u, memory_order_relaxed);
    }
}

constant float3 kBandRgbMsl[8] = {
    float3(255,255,255), float3(0,0,0), float3(128,128,128), float3(255,0,0),
    float3(0,255,0), float3(0,0,255), float3(255,0,255), float3(255,128,0),
};

kernel void b2_pattern(texture2d<float, access::write> dst [[texture(0)]],
                       constant Params& p [[buffer(0)]],
                       uint2 gid [[thread_position_in_grid]])
{
    if (gid.x >= p.width || gid.y >= p.height) { return; }
    float3 c;
    if (p.matrixMode != 0u)
    {
        uint band = min(gid.x / (p.width / 8u), 7u);
        c = kBandRgbMsl[band];
    }
    else
    {
        uint barX = (p.frameIndex * 37u) % p.width;
        if (gid.x >= barX && gid.x < barX + 120u) { c = float3(255, 0, 255); }
        else if (gid.x < p.width / 2u)            { c = float3(255, 128, 0); }
        else                                      { c = float3(0, 64, 255); }
        uint h = hash32((gid.y * p.width + gid.x) * 2654435761u + p.frameIndex * 97u);
        c.r = clamp(c.r + float(h & 0x7Fu) - 64.0, 0.0, 255.0);
        c.g = clamp(c.g + float((h >> 8) & 0x7Fu) - 64.0, 0.0, 255.0);
        c.b = clamp(c.b + float((h >> 16) & 0x7Fu) - 64.0, 0.0, 255.0);
    }
    dst.write(float4(c / 255.0, 1.0), gid);
}
)MSL";

// ---------------------------------------------------------------------------
// B2 decode-and-verify (adapted from vt-llrc-probe): decode the captured
// bitstream in-process, report plane stats / chroma verdict, and in --matrix
// mode compare per-band means against the BT.709 limited-range reference
// (tolerance 8).
// ---------------------------------------------------------------------------

struct EncodedStream
{
    std::vector<std::vector<uint8_t>> annexb;
    CMFormatDescriptionRef formatDesc = nullptr;
    int callbackCount = 0;
    int droppedCount = 0;
    int errorCount = 0;
    int64_t totalBytes = 0;
    std::vector<int64_t> idrPts;
};

static const int kBands = 8;
static const uint8_t kBandRgb[kBands][3] = {
    { 255, 255, 255 }, { 0, 0, 0 },     { 128, 128, 128 }, { 255, 0, 0 },
    { 0, 255, 0 },     { 0, 0, 255 },   { 255, 0, 255 },   { 255, 128, 0 },
};

struct BandAccum
{
    double y = 0, cb = 0, cr = 0;
    int64_t yn = 0, cn = 0;
};
static BandAccum gBands[kBands];

// BT.709 video-range RGB->YCbCr reference (same as vt-llrc-probe).
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
    for (int yy = 0; yy < h; yy++)
    {
        const uint8_t* row = yBase + yy * yStride;
        for (int xx = 0; xx < w; xx += 7)
        {
            int v = row[xx];
            stats->y.minv = std::min(stats->y.minv, v);
            stats->y.maxv = std::max(stats->y.maxv, v);
            ySum += v;
            yCount++;
        }
    }
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
            cbSum += cb;
            crSum += cr;
            cbCount++;
            crCount++;
        }
    }
    if (gMatrixMode)
    {
        const int bandW = w / kBands;
        const int lumaMargin = 24;
        for (int band = 0; band < kBands; band++)
        {
            for (int yy = 0; yy < h; yy += 3)
            {
                const uint8_t* row = yBase + yy * yStride;
                for (int xx = band * bandW + lumaMargin; xx < (band + 1) * bandW - lumaMargin;
                     xx += 3)
                {
                    gBands[band].y += row[xx];
                    gBands[band].yn++;
                }
            }
        }
        const int cBandW = cw / kBands;
        const int chromaMargin = 12;
        for (int band = 0; band < kBands; band++)
        {
            for (int yy = 0; yy < ch; yy += 3)
            {
                const uint8_t* row = cBase + yy * cStride;
                for (int xx = band * cBandW + chromaMargin; xx < (band + 1) * cBandW - chromaMargin;
                     xx += 3)
                {
                    gBands[band].cb += row[xx * 2 + 0];
                    gBands[band].cr += row[xx * 2 + 1];
                    gBands[band].cn++;
                }
            }
        }
    }
    stats->framesDecoded++;
    stats->y.mean = yCount ? ySum / yCount : 0;
    stats->cb.mean = cbCount ? cbSum / cbCount : 0;
    stats->cr.mean = crCount ? crSum / crCount : 0;
    CVPixelBufferUnlockBaseAddress(pb, kCVPixelBufferLock_ReadOnly);
}

// Consumes stream.formatDesc. Returns decoded-frame count via outDecoded.
static bool DecodeAndVerify(EncodedStream& stream, int* outDecoded)
{
    if (stream.formatDesc == nullptr || stream.annexb.empty())
    {
        printf("[child]   NO OUTPUT — encoder produced nothing\n");
        return false;
    }
    for (auto& band : gBands)
    {
        band = BandAccum{};
    }
    DecodeStats dstats;
    VTDecompressionOutputCallbackRecord cb = { DecodeOutput, &dstats };
    NSDictionary* outAttrs = @{
        (NSString*)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
    };
    VTDecompressionSessionRef dec = nullptr;
    OSStatus status = VTDecompressionSessionCreate(nullptr, stream.formatDesc, nullptr,
                                                   (__bridge CFDictionaryRef)outAttrs, &cb, &dec);
    if (status != noErr)
    {
        printf("[child]   DECODER CREATE FAILED: %d\n", (int)status);
        CFRelease(stream.formatDesc);
        stream.formatDesc = nullptr;
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
        CMSampleBufferCreateReady(nullptr, block, stream.formatDesc, 1, 0, nullptr, 1, sizes,
                                  &sample);
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
    stream.formatDesc = nullptr;

    printf("[child]   decoded %d frames\n", dstats.framesDecoded);
    printf("[child]   Y : min=%3d max=%3d mean=%6.1f\n", dstats.y.minv, dstats.y.maxv,
           dstats.y.mean);
    printf("[child]   Cb: min=%3d max=%3d mean=%6.1f\n", dstats.cb.minv, dstats.cb.maxv,
           dstats.cb.mean);
    printf("[child]   Cr: min=%3d max=%3d mean=%6.1f\n", dstats.cr.minv, dstats.cr.maxv,
           dstats.cr.mean);
    if (outDecoded != nullptr)
    {
        *outDecoded = dstats.framesDecoded;
    }
    const bool chromaDead = dstats.cb.maxv <= 20 && dstats.cr.maxv <= 20;
    const bool chromaHealthy = (dstats.cb.maxv - dstats.cb.minv) > 60 &&
                               (dstats.cr.maxv - dstats.cr.minv) > 60;
    printf("[child]   CHROMA: %s\n", chromaDead      ? "DEAD (green bug)"
                                     : chromaHealthy ? "healthy"
                                                     : "SUSPICIOUS (low variance)");
    if (gMatrixMode)
    {
        const double kTolerance = 8.0;
        double worst = 0.0;
        printf("[child]   band      expected Y/Cb/Cr    decoded Y/Cb/Cr       delta\n");
        for (int band = 0; band < kBands; band++)
        {
            uint8_t eY, eCb, eCr;
            Rgb709(kBandRgb[band][0], kBandRgb[band][1], kBandRgb[band][2], eY, eCb, eCr);
            double dY = gBands[band].yn ? gBands[band].y / gBands[band].yn : -1;
            double dCb = gBands[band].cn ? gBands[band].cb / gBands[band].cn : -1;
            double dCr = gBands[band].cn ? gBands[band].cr / gBands[band].cn : -1;
            double delta =
                std::max({ std::abs(dY - eY), std::abs(dCb - eCb), std::abs(dCr - eCr) });
            worst = std::max(worst, delta);
            printf("[child]   #%d %3d/%3d/%3d -> %6.1f/%6.1f/%6.1f  max|d|=%5.1f\n", band, eY, eCb,
                   eCr, dY, dCb, dCr, delta);
        }
        printf("[child]   MATRIX VERDICT: %s (worst delta %.1f, tolerance %.0f)\n",
               worst <= kTolerance ? "BT.709 limited-range MATCH" : "MISMATCH",
               worst, kTolerance);
        return worst <= kTolerance;
    }
    return chromaHealthy;
}

// ---------------------------------------------------------------------------
// CHILD (arm64 native — the future encoder helper).
// ---------------------------------------------------------------------------

static int gSock = -1;
static EncodedStream gStream;              // B2 encode collection
static std::vector<uint64_t> gGpuDoneTicks; // B2: parent GPU-done time per pts
static std::vector<double> gVtLat;          // B2: GPU-done -> VT callback, ms
static VTCompressionSessionRef gVtSession = nullptr;
static bool gVtHardware = false;

// LL-RC contract: everything this callback reads (gGpuDoneTicks[pts]) is
// written BEFORE EncodeFrame is called. The reply to the parent frees the
// slot for reuse.
static void B2EncodeOutput(void*, void* frameRefCon, OSStatus status, VTEncodeInfoFlags infoFlags,
                           CMSampleBufferRef sb)
{
    int64_t fi = (int64_t)(intptr_t)frameRefCon;
    uint32_t ok = 1;
    if (status != noErr)
    {
        gStream.errorCount++;
        ok = 0;
        printf("[child] encode cb frame %lld status=%d\n", (long long)fi, (int)status);
    }
    else if ((infoFlags & kVTEncodeInfo_FrameDropped) != 0 || sb == nullptr ||
             !CMSampleBufferDataIsReady(sb))
    {
        gStream.droppedCount++;
    }
    else
    {
        gStream.callbackCount++;
        CFArrayRef atts = CMSampleBufferGetSampleAttachmentsArray(sb, false);
        bool notSync = false;
        if (atts != nullptr && CFArrayGetCount(atts) > 0)
        {
            CFDictionaryRef d = (CFDictionaryRef)CFArrayGetValueAtIndex(atts, 0);
            CFTypeRef v = d ? CFDictionaryGetValue(d, kCMSampleAttachmentKey_NotSync) : nullptr;
            notSync = v != nullptr && CFBooleanGetValue((CFBooleanRef)v);
        }
        if (!notSync)
        {
            gStream.idrPts.push_back(CMSampleBufferGetPresentationTimeStamp(sb).value);
        }
        CMFormatDescriptionRef desc = CMSampleBufferGetFormatDescription(sb);
        if (desc != nullptr && gStream.formatDesc == nullptr)
        {
            gStream.formatDesc = (CMFormatDescriptionRef)CFRetain(desc);
        }
        CMBlockBufferRef block = CMSampleBufferGetDataBuffer(sb);
        size_t length = 0;
        char* data = nullptr;
        if (CMBlockBufferGetDataPointer(block, 0, nullptr, &length, &data) == kCMBlockBufferNoErr)
        {
            gStream.annexb.emplace_back((uint8_t*)data, (uint8_t*)data + length);
            gStream.totalBytes += (int64_t)length;
        }
        if (fi >= 0 && (size_t)fi < gGpuDoneTicks.size() && gGpuDoneTicks[(size_t)fi] != 0)
        {
            // Both sides of this delta are wall nanoseconds (see NowNs).
            gVtLat.push_back((double)(NowNs() - gGpuDoneTicks[(size_t)fi]) / 1e6);
        }
    }
    SockMsg r;
    r.type = kMsgReply;
    r.slot = (uint32_t)(fi % kSlots);
    r.frameIndex = (uint64_t)fi;
    r.ok = ok;
    WriteAll(gSock, &r, sizeof(r));
}

static bool B2CreateSession()
{
    NSDictionary* spec = @{
        (NSString*)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: @YES,
        (NSString*)kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: @YES,
        (NSString*)kVTVideoEncoderSpecification_EnableLowLatencyRateControl: @YES,
    };
    OSStatus status = VTCompressionSessionCreate(
        kCFAllocatorDefault, gWidth, gHeight, kCMVideoCodecType_HEVC,
        (__bridge CFDictionaryRef)spec, nullptr, kCFAllocatorDefault, B2EncodeOutput, nullptr,
        &gVtSession);
    if (status != noErr)
    {
        printf("[child] VT SESSION CREATE FAILED: %d (HEVC HW-required LL-RC)\n", (int)status);
        return false;
    }
    auto setProp = [&](CFStringRef key, CFTypeRef value)
    {
        OSStatus s = VTSessionSetProperty(gVtSession, key, value);
        if (s != noErr)
        {
            printf("[child] prop %s REJECTED (%d)\n", [(__bridge NSString*)key UTF8String],
                   (int)s);
        }
    };
    setProp(kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    setProp(kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    setProp(kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_HEVC_Main_AutoLevel);
    int br = kBitrate;
    CFNumberRef brRef = CFNumberCreate(nullptr, kCFNumberIntType, &br);
    setProp(kVTCompressionPropertyKey_AverageBitRate, brRef);
    CFRelease(brRef);
    setProp(kVTCompressionPropertyKey_PrioritizeEncodingSpeedOverQuality, kCFBooleanTrue);
    int fps = kFps;
    CFNumberRef fpsRef = CFNumberCreate(nullptr, kCFNumberIntType, &fps);
    setProp(kVTCompressionPropertyKey_ExpectedFrameRate, fpsRef);
    CFRelease(fpsRef);
    setProp(kVTCompressionPropertyKey_ColorPrimaries, kCVImageBufferColorPrimaries_ITU_R_709_2);
    setProp(kVTCompressionPropertyKey_TransferFunction,
            kCVImageBufferTransferFunction_ITU_R_709_2);
    setProp(kVTCompressionPropertyKey_YCbCrMatrix, kCVImageBufferYCbCrMatrix_ITU_R_709_2);
    VTCompressionSessionPrepareToEncodeFrames(gVtSession);

    // The session was created with RequireHardwareAcceleratedVideoEncoder=YES,
    // so its existence proves hardware. The UsingHardware... query is
    // informational only — on arm64 LL-RC sessions it can fail (observed
    // kVTPropertyNotSupportedErr while the encoder ID is still *.rtvc).
    gVtHardware = true;
    CFBooleanRef usingHw = nullptr;
    OSStatus hwStatus = VTSessionCopyProperty(
        gVtSession, kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
        kCFAllocatorDefault, &usingHw);
    if (hwStatus == noErr && usingHw != nullptr)
    {
        printf("[child] hardware=%s (required at create)\n",
               CFBooleanGetValue(usingHw) ? "yes" : "NO (software!)");
        CFRelease(usingHw);
    }
    else
    {
        printf("[child] hardware=yes by construction (RequireHW create succeeded; "
               "UsingHardware query status=%d)\n", (int)hwStatus);
    }
    CFStringRef encoderId = nullptr;
    if (VTSessionCopyProperty(gVtSession, kVTCompressionPropertyKey_EncoderID, kCFAllocatorDefault,
                              &encoderId) == noErr && encoderId != nullptr)
    {
        printf("[child] encoder=%s\n", [(__bridge NSString*)encoderId UTF8String]);
        CFRelease(encoderId);
    }
    return true;
}

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static kern_return_t ChildBootstrapRegister(const char* name, mach_port_t port)
{
    return bootstrap_register(bootstrap_port, (char*)name, port);
}
#pragma clang diagnostic pop

static int ChildMain(int sockFd, const char* route, const char* name, bool b2)
{
#if defined(__arm64__)
    printf("[child] arch: arm64 native, pid %d\n", getpid());
#else
    printf("[child] arch: NOT arm64 — build/spawn error\n");
    return 1;
#endif
    gSock = sockFd;
    mach_port_t rx = MACH_PORT_NULL;
    kern_return_t kr = mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &rx);
    if (kr != KERN_SUCCESS)
    {
        printf("[child] mach_port_allocate failed: 0x%x (%s)\n", kr, mach_error_string(kr));
        return 1;
    }
    if (strcmp(route, "special") == 0)
    {
        mach_port_t bs = MACH_PORT_NULL;
        task_get_bootstrap_port(mach_task_self(), &bs);
        kr = SendPortMsg(bs, kMachCheckin, rx, MACH_MSG_TYPE_MAKE_SEND, (uint32_t)getpid(), 0, 0);
        if (kr != KERN_SUCCESS)
        {
            printf("[child] check-in via TASK_BOOTSTRAP_PORT FAILED: 0x%x (%s)\n", kr,
                   mach_error_string(kr));
            return 1;
        }
        printf("[child] checked in via parent-installed TASK_BOOTSTRAP_PORT special port\n");
    }
    else
    {
        kr = mach_port_insert_right(mach_task_self(), rx, rx, MACH_MSG_TYPE_MAKE_SEND);
        if (kr == KERN_SUCCESS)
        {
            kr = ChildBootstrapRegister(name, rx);
        }
        if (kr != KERN_SUCCESS)
        {
            printf("[child] bootstrap_register('%s') FAILED: 0x%x (%s)\n", name, kr,
                   bootstrap_strerror(kr));
            return 1;
        }
        printf("[child] registered '%s' via bootstrap_register (fallback route)\n", name);
    }

    // Watchdog for the special-port route: replacing TASK_BOOTSTRAP_PORT
    // poisons libxpc, which latched the (parent's, non-launchd) bootstrap port
    // during libSystem init — the first XPC-touching call (os_log via
    // IOSurface lookup, MTLCompilerService, VideoToolbox) then hangs forever
    // in bootstrap_look_up2 even after main() restores the real port. Detect
    // the hang and exit so the parent can fall back to bootstrap_register.
    static std::atomic<bool> initDone{ false };
    if (strcmp(route, "special") == 0)
    {
        std::thread(
            []
            {
                for (int i = 0; i < 100 && !initDone.load(); i++)
                {
                    usleep(100 * 1000);
                }
                if (!initDone.load())
                {
                    printf("[child] HUNG in XPC after bootstrap replacement — libxpc latched the "
                           "parent's TASK_BOOTSTRAP_PORT during libSystem init; restoring the "
                           "real port in main() is too late. Special-port route UNUSABLE for a "
                           "Metal/VT child. Exiting for fallback.\n");
                    _exit(3);
                }
            })
            .detach();
    }

    // First message: the REAL bootstrap port. Restore it before Metal/VT —
    // both need XPC service lookups that go through the bootstrap port.
    MachMsgRecv msg;
    kr = RecvPortMsg(rx, msg, 10000);
    if (kr != KERN_SUCCESS || msg.m.hdr.msgh_id != kMachBootstrap)
    {
        printf("[child] bootstrap-restore recv failed: 0x%x id=0x%x\n", kr, msg.m.hdr.msgh_id);
        return 1;
    }
    task_set_special_port(mach_task_self(), TASK_BOOTSTRAP_PORT, msg.m.port.name);
    bootstrap_port = msg.m.port.name;
    printf("[child] real bootstrap port restored — XPC services reachable\n");

    IOSurfaceRef surf[kSlots] = {};
    for (int i = 0; i < kSlots; i++)
    {
        kr = RecvPortMsg(rx, msg, 10000);
        if (kr != KERN_SUCCESS || msg.m.hdr.msgh_id != kMachSurface)
        {
            printf("[child] surface recv %d failed: 0x%x id=0x%x\n", i, kr, msg.m.hdr.msgh_id);
            return 1;
        }
        uint32_t slot = msg.m.a, w = msg.m.b, h = msg.m.c;
        IOSurfaceRef s = IOSurfaceLookupFromMachPort(msg.m.port.name);
        mach_port_deallocate(mach_task_self(), msg.m.port.name);
        if (s == nullptr || slot >= kSlots)
        {
            printf("[child] IOSurfaceLookupFromMachPort FAILED (slot %u)\n", slot);
            return 1;
        }
        OSType fmt = IOSurfaceGetPixelFormat(s);
        size_t sw = IOSurfaceGetWidth(s), sh = IOSurfaceGetHeight(s);
        size_t alloc = IOSurfaceGetAllocSize(s), rb = IOSurfaceGetBytesPerRow(s);
        bool okS = sw == w && sh == h && fmt == 'BGRA' && alloc >= sw * sh * 4;
        printf("[child] surface slot %u: %zux%zu fmt='%c%c%c%c' rowBytes=%zu alloc=%zu %s\n", slot,
               sw, sh, (char)(fmt >> 24), (char)(fmt >> 16), (char)(fmt >> 8), (char)fmt, rb,
               alloc, okS ? "OK" : "INVALID");
        if (!okS)
        {
            return 1;
        }
        surf[slot] = s;
    }

    // Metal (bootstrap is restored, so the XPC shader compiler works).
    id<MTLDevice> device = MTLCreateSystemDefaultDevice();
    if (device == nil)
    {
        printf("[child] NO METAL DEVICE\n");
        return 1;
    }
    NSError* err = nil;
    id<MTLLibrary> lib = [device newLibraryWithSource:[NSString stringWithUTF8String:kMetalSource]
                                              options:nil
                                                error:&err];
    if (lib == nil)
    {
        printf("[child] metal compile failed: %s\n", err.localizedDescription.UTF8String);
        return 1;
    }
    id<MTLComputePipelineState> verifyPso =
        [device newComputePipelineStateWithFunction:[lib newFunctionWithName:@"verify_pattern"]
                                              error:&err];
    if (verifyPso == nil)
    {
        printf("[child] verify pipeline failed: %s\n", err.localizedDescription.UTF8String);
        return 1;
    }
    id<MTLCommandQueue> queue = [device newCommandQueue];
    id<MTLTexture> tex[kSlots];
    MTLTextureDescriptor* td =
        [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                           width:(NSUInteger)gWidth
                                                          height:(NSUInteger)gHeight
                                                       mipmapped:NO];
    td.usage = MTLTextureUsageShaderRead;
    td.storageMode = MTLStorageModeShared;
    for (int i = 0; i < kSlots; i++)
    {
        tex[i] = [device newTextureWithDescriptor:td iosurface:surf[i] plane:0];
        if (tex[i] == nil)
        {
            printf("[child] MTLTexture from IOSurface failed (slot %d)\n", i);
            return 1;
        }
    }

    CVPixelBufferRef wrapped[kSlots] = {};
    if (b2)
    {
        // The production seam: wrap each foreign IOSurface ONCE, reuse forever.
        for (int i = 0; i < kSlots; i++)
        {
            CVReturn cvr = CVPixelBufferCreateWithIOSurface(kCFAllocatorDefault, surf[i], nullptr,
                                                            &wrapped[i]);
            if (cvr != kCVReturnSuccess)
            {
                printf("[child] CVPixelBufferCreateWithIOSurface FAILED: %d (slot %d)\n", cvr, i);
                return 1;
            }
            CVBufferSetAttachment(wrapped[i], kCVImageBufferColorPrimariesKey,
                                  kCVImageBufferColorPrimaries_ITU_R_709_2,
                                  kCVAttachmentMode_ShouldPropagate);
            CVBufferSetAttachment(wrapped[i], kCVImageBufferTransferFunctionKey,
                                  kCVImageBufferTransferFunction_ITU_R_709_2,
                                  kCVAttachmentMode_ShouldPropagate);
            CVBufferSetAttachment(wrapped[i], kCVImageBufferYCbCrMatrixKey,
                                  kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                                  kCVAttachmentMode_ShouldPropagate);
        }
        printf("[child] wrapped %d foreign IOSurfaces as CVPixelBuffers (once, reused)\n", kSlots);
        gGpuDoneTicks.assign(1 << 20, 0);
        if (!B2CreateSession())
        {
            return 1;
        }
    }

    id<MTLBuffer> mbuf = [device newBufferWithLength:4 options:MTLResourceStorageModeShared];
    initDone = true;
    SockMsg ready;
    ready.type = kMsgReady;
    ready.ok = 1;
    WriteAll(gSock, &ready, sizeof(ready));

    SockMsg m;
    while (ReadAll(gSock, &m, sizeof(m)))
    {
        if (m.type == kMsgSubmit && !b2)
        {
            // Child-side GPU verification of the full surface for exactly
            // frame m.frameIndex (no CPU lock of the surface).
            *(uint32_t*)mbuf.contents = 0;
            id<MTLCommandBuffer> cmd = [queue commandBuffer];
            id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
            [enc setComputePipelineState:verifyPso];
            [enc setTexture:tex[m.slot % kSlots] atIndex:0];
            GpuParams params = { (uint32_t)gWidth, (uint32_t)gHeight, (uint32_t)m.frameIndex, 0 };
            [enc setBytes:&params length:sizeof(params) atIndex:0];
            [enc setBuffer:mbuf offset:0 atIndex:1];
            [enc dispatchThreadgroups:MTLSizeMake((gWidth + 15) / 16, (gHeight + 15) / 16, 1)
                threadsPerThreadgroup:MTLSizeMake(16, 16, 1)];
            [enc endEncoding];
            [cmd commit];
            [cmd waitUntilCompleted];
            uint32_t mm = *(uint32_t*)mbuf.contents;
            if (mm != 0)
            {
                printf("[child] PATTERN MISMATCH frame %llu slot %u: %u pixels\n",
                       (unsigned long long)m.frameIndex, m.slot, mm);
            }
            SockMsg r;
            r.type = kMsgReply;
            r.slot = m.slot;
            r.frameIndex = m.frameIndex;
            r.ok = mm == 0 ? 1u : 0u;
            r.mismatches = mm;
            if (!WriteAll(gSock, &r, sizeof(r)))
            {
                break;
            }
        }
        else if (m.type == kMsgSubmit && b2)
        {
            // ALL bookkeeping BEFORE EncodeFrame: the LL-RC callback can fire
            // before EncodeFrame returns.
            if (m.frameIndex < gGpuDoneTicks.size())
            {
                gGpuDoneTicks[m.frameIndex] = m.tGpuDone;
            }
            OSStatus s = VTCompressionSessionEncodeFrame(
                gVtSession, wrapped[m.slot % kSlots], CMTimeMake((int64_t)m.frameIndex, kFps),
                kCMTimeInvalid, nullptr, (void*)(intptr_t)m.frameIndex, nullptr);
            if (s != noErr)
            {
                printf("[child] EncodeFrame FAILED frame %llu: %d\n",
                       (unsigned long long)m.frameIndex, (int)s);
                SockMsg r;
                r.type = kMsgReply;
                r.slot = m.slot;
                r.frameIndex = m.frameIndex;
                r.ok = 0;
                WriteAll(gSock, &r, sizeof(r));
            }
        }
        else if (m.type == kMsgEnd)
        {
            uint32_t finalOk = 1;
            if (b2)
            {
                VTCompressionSessionCompleteFrames(gVtSession, kCMTimeInvalid);
                int total = (int)m.frameIndex;
                int accounted = gStream.callbackCount + gStream.droppedCount + gStream.errorCount;
                printf("[child] encoded: callbacks=%d dropped=%d errors=%d of %d submitted "
                       "avgKB/frame=%.0f (%.1f Mbps @%dfps)\n",
                       gStream.callbackCount, gStream.droppedCount, gStream.errorCount, total,
                       gStream.annexb.empty() ? 0.0
                                              : gStream.totalBytes / 1024.0 / gStream.annexb.size(),
                       gStream.annexb.empty()
                           ? 0.0
                           : gStream.totalBytes * 8.0 * kFps / gStream.annexb.size() / 1e6,
                       kFps);
                printf("[child] sync samples (pts):");
                for (int64_t p : gStream.idrPts)
                {
                    printf(" %lld", (long long)p);
                }
                printf("\n");
                PrintPercentiles("[child] GPU-done -> child VT callback", gVtLat);
                int decoded = 0;
                bool decodeOk = DecodeAndVerify(gStream, &decoded);
                bool countsOk = accounted == total && decoded == gStream.callbackCount &&
                                gStream.errorCount == 0;
                finalOk = (decodeOk && countsOk && gVtHardware) ? 1u : 0u;
                printf("[child] B2 frame accounting: %d accounted of %d submitted, %d decoded "
                       "of %d emitted -> %s\n",
                       accounted, total, decoded, gStream.callbackCount,
                       countsOk ? "OK" : "MISMATCH");
                VTCompressionSessionInvalidate(gVtSession);
                CFRelease(gVtSession);
                gVtSession = nullptr;
            }
            SockMsg r;
            r.type = kMsgEndAck;
            r.ok = finalOk;
            WriteAll(gSock, &r, sizeof(r));
            break;
        }
    }
    for (int i = 0; i < kSlots; i++)
    {
        if (wrapped[i] != nullptr)
        {
            CVPixelBufferRelease(wrapped[i]);
        }
        if (surf[i] != nullptr)
        {
            CFRelease(surf[i]);
        }
    }
    return 0;
}

// ---------------------------------------------------------------------------
// PARENT (x86_64 under Rosetta — simulating the Wine-side runtime).
// ---------------------------------------------------------------------------

struct ParentState
{
    std::string childBin;
    bool b2 = false;
    const char* route = "special";
    char name[64] = {};
    int spawnSeq = 0;
    mach_port_t rxPort = MACH_PORT_NULL; // receive right, allocated once
    mach_port_t childPort = MACH_PORT_NULL;
    pid_t childPid = -1;
    int sock = -1;
    CVPixelBufferRef pbs[kSlots] = {};
    CVMetalTextureRef cvTex[kSlots] = {};
    id<MTLTexture> tex[kSlots];
    CVMetalTextureCacheRef texCache = nullptr;
    id<MTLDevice> device;
    id<MTLCommandQueue> queue;
    id<MTLComputePipelineState> writePso;
};

static bool ParentMetalSetup(ParentState& st)
{
    st.device = MTLCreateSystemDefaultDevice();
    if (st.device == nil)
    {
        printf("NO METAL DEVICE\n");
        return false;
    }
    NSError* err = nil;
    id<MTLLibrary> lib =
        [st.device newLibraryWithSource:[NSString stringWithUTF8String:kMetalSource]
                                options:nil
                                  error:&err];
    if (lib == nil)
    {
        printf("metal compile failed: %s\n", err.localizedDescription.UTF8String);
        return false;
    }
    st.writePso = [st.device
        newComputePipelineStateWithFunction:[lib newFunctionWithName:st.b2 ? @"b2_pattern"
                                                                           : @"write_pattern"]
                                      error:&err];
    if (st.writePso == nil)
    {
        printf("write pipeline failed: %s\n", err.localizedDescription.UTF8String);
        return false;
    }
    st.queue = [st.device newCommandQueue];
    if (CVMetalTextureCacheCreate(kCFAllocatorDefault, nullptr, st.device, nullptr,
                                  &st.texCache) != kCVReturnSuccess)
    {
        printf("CVMetalTextureCache create failed\n");
        return false;
    }
    NSDictionary* pbAttrs = @{
        (NSString*)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (NSString*)kCVPixelBufferIOSurfacePropertiesKey: @{},
        (NSString*)kCVPixelBufferMetalCompatibilityKey: @YES,
    };
    NSDictionary* texAttrs = @{
        (NSString*)kCVMetalTextureUsage: @(MTLTextureUsageShaderRead | MTLTextureUsageShaderWrite),
    };
    for (int s = 0; s < kSlots; s++)
    {
        if (CVPixelBufferCreate(nullptr, gWidth, gHeight, kCVPixelFormatType_32BGRA,
                                (__bridge CFDictionaryRef)pbAttrs, &st.pbs[s]) != kCVReturnSuccess)
        {
            printf("pixel buffer create failed (slot %d)\n", s);
            return false;
        }
        CVBufferSetAttachment(st.pbs[s], kCVImageBufferColorPrimariesKey,
                              kCVImageBufferColorPrimaries_ITU_R_709_2,
                              kCVAttachmentMode_ShouldPropagate);
        CVBufferSetAttachment(st.pbs[s], kCVImageBufferTransferFunctionKey,
                              kCVImageBufferTransferFunction_ITU_R_709_2,
                              kCVAttachmentMode_ShouldPropagate);
        CVBufferSetAttachment(st.pbs[s], kCVImageBufferYCbCrMatrixKey,
                              kCVImageBufferYCbCrMatrix_ITU_R_709_2,
                              kCVAttachmentMode_ShouldPropagate);
        if (CVPixelBufferGetIOSurface(st.pbs[s]) == nullptr)
        {
            printf("pixel buffer has no IOSurface (slot %d)\n", s);
            return false;
        }
        if (CVMetalTextureCacheCreateTextureFromImage(
                kCFAllocatorDefault, st.texCache, st.pbs[s], (__bridge CFDictionaryRef)texAttrs,
                MTLPixelFormatBGRA8Unorm, gWidth, gHeight, 0, &st.cvTex[s]) != kCVReturnSuccess)
        {
            printf("CVMetalTexture create failed (slot %d)\n", s);
            return false;
        }
        st.tex[s] = CVMetalTextureGetTexture(st.cvTex[s]);
    }
    return true;
}

static bool SpawnChild(ParentState& st)
{
    int fds[2];
    if (socketpair(AF_UNIX, SOCK_STREAM, 0, fds) != 0)
    {
        printf("socketpair failed: errno=%d (%s)\n", errno, strerror(errno));
        return false;
    }
    int one = 1;
    setsockopt(fds[0], SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
    setsockopt(fds[1], SOL_SOCKET, SO_NOSIGPIPE, &one, sizeof(one));
    snprintf(st.name, sizeof(st.name), "xarch-ipc-probe.%d.%d", getpid(), st.spawnSeq++);
    char fdStr[16], sizeStr[32];
    snprintf(fdStr, sizeof(fdStr), "%d", fds[1]);
    snprintf(sizeStr, sizeof(sizeStr), "%dx%d", gWidth, gHeight);
    std::vector<const char*> argv = { st.childBin.c_str(), "--child",  "--sock-fd", fdStr,
                                      "--route",           st.route,   "--name",    st.name,
                                      "--size",            sizeStr };
    if (st.b2)
    {
        argv.push_back("--b2");
    }
    if (gMatrixMode)
    {
        argv.push_back("--matrix");
    }
    argv.push_back(nullptr);
    posix_spawnattr_t attr;
    posix_spawnattr_init(&attr);
    if (strcmp(st.route, "special") == 0)
    {
        kern_return_t kr = posix_spawnattr_setspecialport_np(&attr, st.rxPort,
                                                             TASK_BOOTSTRAP_PORT);
        if (kr != KERN_SUCCESS)
        {
            printf("posix_spawnattr_setspecialport_np FAILED: 0x%x (%s)\n", kr,
                   mach_error_string(kr));
            posix_spawnattr_destroy(&attr);
            close(fds[0]);
            close(fds[1]);
            return false;
        }
    }
    int rc = posix_spawn(&st.childPid, st.childBin.c_str(), nullptr, &attr,
                         (char* const*)argv.data(), environ);
    posix_spawnattr_destroy(&attr);
    close(fds[1]);
    if (rc != 0)
    {
        printf("posix_spawn('%s') failed: errno=%d (%s)\n", st.childBin.c_str(), rc,
               strerror(rc));
        close(fds[0]);
        st.childPid = -1;
        return false;
    }
    st.sock = fds[0];
    return true;
}

static void CleanupChild(ParentState& st, bool killIt)
{
    if (killIt && st.childPid > 0)
    {
        kill(st.childPid, SIGKILL);
    }
    if (st.sock >= 0)
    {
        close(st.sock);
        st.sock = -1;
    }
    if (st.childPid > 0)
    {
        int status = 0;
        waitpid(st.childPid, &status, 0);
        st.childPid = -1;
    }
    if (st.childPort != MACH_PORT_NULL)
    {
        mach_port_deallocate(mach_task_self(), st.childPort);
        st.childPort = MACH_PORT_NULL;
    }
}

// Spawn + rendezvous + bootstrap-restore + surface registration + ready.
static bool SpawnAndRendezvous(ParentState& st)
{
    if (!SpawnChild(st))
    {
        return false;
    }
    if (strcmp(st.route, "special") == 0)
    {
        MachMsgRecv msg;
        kern_return_t kr = RecvPortMsg(st.rxPort, msg, 8000);
        if (kr != KERN_SUCCESS || msg.m.hdr.msgh_id != kMachCheckin)
        {
            printf("special-port check-in recv FAILED: 0x%x (%s) id=0x%x\n", kr,
                   mach_error_string(kr), msg.m.hdr.msgh_id);
            CleanupChild(st, true);
            return false;
        }
        st.childPort = msg.m.port.name;
    }
    else
    {
        kern_return_t kr = 1;
        for (int i = 0; i < 160 && kr != KERN_SUCCESS; i++)
        {
            kr = bootstrap_look_up(bootstrap_port, st.name, &st.childPort);
            if (kr != KERN_SUCCESS)
            {
                usleep(50 * 1000);
            }
        }
        if (kr != KERN_SUCCESS)
        {
            printf("bootstrap_look_up('%s') FAILED: 0x%x (%s)\n", st.name, kr,
                   bootstrap_strerror(kr));
            CleanupChild(st, true);
            return false;
        }
    }
    kern_return_t kr = SendPortMsg(st.childPort, kMachBootstrap, bootstrap_port,
                                   MACH_MSG_TYPE_COPY_SEND, 0, 0, 0);
    if (kr != KERN_SUCCESS)
    {
        printf("bootstrap-restore send FAILED: 0x%x\n", kr);
        CleanupChild(st, true);
        return false;
    }
    for (int s = 0; s < kSlots; s++)
    {
        mach_port_t sp = IOSurfaceCreateMachPort(CVPixelBufferGetIOSurface(st.pbs[s]));
        // MOVE_SEND: the right transfers to the child; no parent-side leak.
        kr = SendPortMsg(st.childPort, kMachSurface, sp, MACH_MSG_TYPE_MOVE_SEND, (uint32_t)s,
                         (uint32_t)gWidth, (uint32_t)gHeight);
        if (kr != KERN_SUCCESS)
        {
            printf("surface %d send FAILED: 0x%x\n", s, kr);
            CleanupChild(st, true);
            return false;
        }
    }
    SockMsg ready;
    if (!ReadAll(st.sock, &ready, sizeof(ready)) || ready.type != kMsgReady || ready.ok != 1)
    {
        printf("child never signalled READY\n");
        CleanupChild(st, true);
        return false;
    }
    return true;
}

struct RotResult
{
    uint64_t completed = 0;
    uint64_t stale = 0;
    bool aborted = false;
};

// GPU-write frames into rotating slots, notify the child over the socket from
// the command buffer's completed handler, and consume in-order verified
// replies. killAt >= 0: SIGKILL the child after submitting that many frames
// (in-flight frames pending) — the crash-audit path.
static RotResult RunRotations(ParentState& st, uint64_t count, int64_t killAt,
                              std::vector<double>* latencies, bool pace)
{
    RotResult res;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    for (int s = 0; s < kSlots; s++)
    {
        dispatch_semaphore_signal(sem);
    }
    std::vector<uint64_t> sendTimes(count, 0);
    std::atomic<bool> dead{ false };
    std::atomic<int> pending{ 0 };
    int sockFd = st.sock;

    std::thread reader(
        [&]
        {
            uint64_t expect = 0;
            SockMsg r;
            while (expect < count)
            {
                if (!ReadAll(sockFd, &r, sizeof(r)))
                {
                    dead = true;
                    break;
                }
                uint64_t now = mach_absolute_time();
                if (r.type != kMsgReply)
                {
                    continue;
                }
                if (r.frameIndex != expect || r.ok != 1)
                {
                    res.stale++;
                    printf("  STALE/MISMATCH reply: frame %llu (expect %llu) ok=%u "
                           "mismatchedPixels=%u\n",
                           (unsigned long long)r.frameIndex, (unsigned long long)expect, r.ok,
                           r.mismatches);
                    dead = true;
                    break;
                }
                if (latencies != nullptr && sendTimes[r.frameIndex] != 0)
                {
                    latencies->push_back(TicksToMs(now - sendTimes[r.frameIndex]));
                }
                res.completed++;
                expect++;
                dispatch_semaphore_signal(sem);
            }
            for (int s = 0; s < kSlots + 1; s++)
            {
                dispatch_semaphore_signal(sem); // unblock main on any exit
            }
        });

    uint64_t* stp = sendTimes.data();
    bool killed = false;
    for (uint64_t i = 0; i < count; i++)
    {
        if (killAt >= 0 && (int64_t)i == killAt)
        {
            printf("  SIGKILL child pid %d after %llu submits (frames in flight)\n", st.childPid,
                   (unsigned long long)i);
            kill(st.childPid, SIGKILL);
            killed = true;
            res.aborted = true;
            break;
        }
        if (dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)) != 0)
        {
            printf("  TIMEOUT waiting for a free slot at frame %llu\n", (unsigned long long)i);
            res.aborted = true;
            break;
        }
        if (dead)
        {
            res.aborted = true;
            break;
        }
        int s = (int)(i % kSlots);
        id<MTLCommandBuffer> cmd = [st.queue commandBuffer];
        id<MTLComputeCommandEncoder> enc = [cmd computeCommandEncoder];
        [enc setComputePipelineState:st.writePso];
        [enc setTexture:st.tex[s] atIndex:0];
        GpuParams params = { (uint32_t)gWidth, (uint32_t)gHeight, (uint32_t)i,
                             gMatrixMode ? 1u : 0u };
        [enc setBytes:&params length:sizeof(params) atIndex:0];
        [enc dispatchThreadgroups:MTLSizeMake((gWidth + 15) / 16, (gHeight + 15) / 16, 1)
            threadsPerThreadgroup:MTLSizeMake(16, 16, 1)];
        [enc endEncoding];
        const uint64_t fi = i;
        const uint32_t slot = (uint32_t)s;
        std::atomic<int>* pendingPtr = &pending;
        pending++;
        [cmd addCompletedHandler:^(id<MTLCommandBuffer>) {
            uint64_t t = mach_absolute_time();
            stp[fi] = t;
            SockMsg sm;
            sm.type = kMsgSubmit;
            sm.slot = slot;
            sm.frameIndex = fi;
            // Wire timestamps are nanoseconds — parent ticks (Rosetta 1/1) and
            // child ticks (arm64 125/3) are different clock domains.
            sm.tGpuDone = t * gTimebase.numer / gTimebase.denom;
            WriteAll(sockFd, &sm, sizeof(sm)); // failure OK if child died
            (*pendingPtr)--;
        }];
        [cmd commit];
        if (pace)
        {
            std::this_thread::sleep_for(std::chrono::microseconds(1000000 / kFps));
        }
    }
    // Drain outstanding completed handlers before sendTimes goes away.
    for (int spins = 0; pending.load() > 0 && spins < 5000; spins++)
    {
        usleep(1000);
    }
    if (killed)
    {
        int status = 0;
        waitpid(st.childPid, &status, 0);
        st.childPid = -1;
    }
    if (res.aborted)
    {
        shutdown(sockFd, SHUT_RDWR); // force reader EOF
    }
    reader.join();
    return res;
}

static bool SendEnd(ParentState& st, uint64_t totalFrames, uint32_t* ackOk)
{
    SockMsg end;
    end.type = kMsgEnd;
    end.frameIndex = totalFrames;
    if (!WriteAll(st.sock, &end, sizeof(end)))
    {
        return false;
    }
    SockMsg ack;
    if (!ReadAll(st.sock, &ack, sizeof(ack)) || ack.type != kMsgEndAck)
    {
        return false;
    }
    if (ackOk != nullptr)
    {
        *ackOk = ack.ok;
    }
    return true;
}

// Try the special-port route, fall back to bootstrap_register. Prints which
// route worked — plan-critical result.
static bool Rendezvous(ParentState& st)
{
    st.route = "special";
    if (SpawnAndRendezvous(st))
    {
        return true;
    }
    printf("special-port route FAILED — trying bootstrap_register fallback\n");
    st.route = "bootstrap";
    return SpawnAndRendezvous(st);
}

static int GateBMain(ParentState& st, uint64_t rotations)
{
    printf("\n=== GATE B: cross-arch rendezvous + IOSurface rotation ===\n");
    if (!Rendezvous(st))
    {
        printf("\nGATE B VERDICT: FAIL — no rendezvous route worked\n");
        return 1;
    }
    printf("RENDEZVOUS ROUTE: %s (%s)\n", st.route,
           strcmp(st.route, "special") == 0
               ? "posix_spawnattr_setspecialport_np TASK_BOOTSTRAP_PORT"
               : "bootstrap_register/bootstrap_look_up");

    std::vector<double> lat;
    lat.reserve(rotations);
    printf("rotation stress: %llu rotations over %d slots, full-surface child GPU verify...\n",
           (unsigned long long)rotations, kSlots);
    RotResult r = RunRotations(st, rotations, -1, &lat, false);
    bool rotOk = r.completed == rotations && r.stale == 0 && !r.aborted;
    printf("rotations: %llu/%llu verified, %llu stale/mismatched\n",
           (unsigned long long)r.completed, (unsigned long long)rotations,
           (unsigned long long)r.stale);
    PrintPercentiles("notify+verify round-trip", lat);
    SendEnd(st, rotations, nullptr);
    CleanupChild(st, false);

    printf("\ncrash/leak audit: 5x SIGKILL mid-stream -> respawn -> re-register -> continue\n");
    int basePorts = CountPorts();
    printf("baseline parent mach port count: %d\n", basePorts);
    bool cyclesOk = true;
    std::vector<int> portCounts;
    for (int cycle = 1; cycle <= 5; cycle++)
    {
        if (!SpawnAndRendezvous(st))
        {
            printf("cycle %d: respawn/rendezvous FAILED\n", cycle);
            cyclesOk = false;
            break;
        }
        RunRotations(st, 1000, 500, nullptr, false); // dies mid-stream
        CleanupChild(st, false);                     // already dead; reap + dealloc
        // Same surfaces, next child: proves parent's IOSurfaces survive.
        if (!SpawnAndRendezvous(st))
        {
            printf("cycle %d: post-kill respawn FAILED\n", cycle);
            cyclesOk = false;
            break;
        }
        RotResult c = RunRotations(st, 200, -1, nullptr, false);
        SendEnd(st, 200, nullptr);
        CleanupChild(st, false);
        bool contOk = c.completed == 200 && c.stale == 0;
        cyclesOk = cyclesOk && contOk;
        int pc = CountPorts();
        portCounts.push_back(pc);
        printf("cycle %d: post-respawn rotations %llu/200 (%s), parent mach ports=%d\n", cycle,
               (unsigned long long)c.completed, contOk ? "ok" : "FAIL", pc);
    }
    bool monotonicGrowth = portCounts.size() == 5;
    for (size_t i = 1; i < portCounts.size() && monotonicGrowth; i++)
    {
        monotonicGrowth = portCounts[i] > portCounts[i - 1];
    }
    bool portsOk = portCounts.size() == 5 && !monotonicGrowth;
    printf("port-count trend: %s\n",
           portsOk ? "stable (no monotonic growth)" : "MONOTONIC GROWTH — leak suspected");

    bool pass = rotOk && cyclesOk && portsOk;
    printf("\nGATE B VERDICT: %s (route=%s, %llu/%llu rotations, %llu stale, kill/respawn %s, "
           "ports %s)\n",
           pass ? "PASS" : "FAIL", st.route, (unsigned long long)r.completed,
           (unsigned long long)rotations, (unsigned long long)r.stale,
           cyclesOk ? "clean" : "FAILED", portsOk ? "stable" : "leaking");
    return pass ? 0 : 1;
}

static int GateB2Main(ParentState& st, uint64_t frames)
{
    printf("\n=== GATE B2: foreign-surface VT seam (HEVC HW LL-RC in arm64 child)%s ===\n",
           gMatrixMode ? " [BT.709 matrix content]" : "");
    if (!Rendezvous(st))
    {
        printf("\nGATE B2 VERDICT: FAIL — no rendezvous route worked\n");
        return 1;
    }
    printf("RENDEZVOUS ROUTE: %s\n", st.route);
    std::vector<double> lat;
    lat.reserve(frames);
    RotResult r = RunRotations(st, frames, -1, &lat, true /* 72fps cadence */);
    PrintPercentiles("GPU-done -> child-VT-callback reply RTT (parent view)", lat);
    uint32_t ackOk = 0;
    bool ackReceived = SendEnd(st, frames, &ackOk);
    CleanupChild(st, false);
    bool pass = ackReceived && ackOk == 1 && r.completed == frames && r.stale == 0 && !r.aborted;
    printf("\nGATE B2 VERDICT: %s (%llu/%llu frames, child verdict %s)\n", pass ? "PASS" : "FAIL",
           (unsigned long long)r.completed, (unsigned long long)frames,
           !ackReceived ? "MISSING" : (ackOk == 1 ? "ok" : "FAIL"));
    return pass ? 0 : 1;
}

int main(int argc, char** argv)
{
    setvbuf(stdout, nullptr, _IOLBF, 0);
    mach_timebase_info(&gTimebase);
    bool child = false, b2 = false;
    int sockFd = -1;
    const char* route = "special";
    const char* name = "";
    uint64_t rotations = 100000;
    uint64_t frames = 300;
    for (int i = 1; i < argc; i++)
    {
        if (strcmp(argv[i], "--child") == 0)
        {
            child = true;
        }
        else if (strcmp(argv[i], "--b2") == 0)
        {
            b2 = true;
        }
        else if (strcmp(argv[i], "--matrix") == 0)
        {
            gMatrixMode = true;
        }
        else if (strcmp(argv[i], "--sock-fd") == 0 && i + 1 < argc)
        {
            sockFd = atoi(argv[++i]);
        }
        else if (strcmp(argv[i], "--route") == 0 && i + 1 < argc)
        {
            route = argv[++i];
        }
        else if (strcmp(argv[i], "--name") == 0 && i + 1 < argc)
        {
            name = argv[++i];
        }
        else if (strcmp(argv[i], "--rotations") == 0 && i + 1 < argc)
        {
            rotations = strtoull(argv[++i], nullptr, 10);
        }
        else if (strcmp(argv[i], "--frames") == 0 && i + 1 < argc)
        {
            frames = strtoull(argv[++i], nullptr, 10);
        }
        else if (strcmp(argv[i], "--size") == 0 && i + 1 < argc)
        {
            int w = 0, h = 0;
            if (sscanf(argv[++i], "%dx%d", &w, &h) == 2 && w > 0 && h > 0)
            {
                gWidth = w & ~1;
                gHeight = h & ~1;
            }
        }
        else
        {
            fprintf(stderr,
                    "usage: %s [--b2] [--matrix] [--rotations N] [--frames N] [--size WxH]\n",
                    argv[0]);
            return 2;
        }
    }
    @autoreleasepool
    {
        if (child)
        {
            return ChildMain(sockFd, route, name, b2);
        }
#if defined(__x86_64__)
        printf("parent arch: x86_64 (Rosetta — simulating the Wine runtime)\n");
#else
        printf("parent arch: arm64 — NOT the cross-arch shape this gate certifies!\n");
#endif
        printf("dims: %dx%d, slots=%d\n", gWidth, gHeight, kSlots);
        ParentState st;
        st.b2 = b2;
        char binPath[4096];
        uint32_t sz = sizeof(binPath);
        std::string self = argv[0];
        if (realpath(argv[0], binPath) != nullptr)
        {
            self = binPath;
        }
        (void)sz;
        st.childBin = self + "-arm64";
        if (access(st.childBin.c_str(), X_OK) != 0)
        {
            printf("child binary '%s' missing — build the arm64 variant first\n",
                   st.childBin.c_str());
            return 1;
        }
        kern_return_t kr = mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE,
                                              &st.rxPort);
        if (kr == KERN_SUCCESS)
        {
            kr = mach_port_insert_right(mach_task_self(), st.rxPort, st.rxPort,
                                        MACH_MSG_TYPE_MAKE_SEND);
        }
        if (kr != KERN_SUCCESS)
        {
            printf("rendezvous port allocate failed: 0x%x (%s)\n", kr, mach_error_string(kr));
            return 1;
        }
        if (!ParentMetalSetup(st))
        {
            return 1;
        }
        return b2 ? GateB2Main(st, frames) : GateBMain(st, rotations);
    }
}
