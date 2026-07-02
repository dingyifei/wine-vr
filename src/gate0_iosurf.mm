// gate0_iosurf.mm — Gate 0 native IOSurface + MTLSharedEvent mechanism probe (no Wine).
//
// Proves the core primitive the zero-copy wineopenxr path depends on:
//   Metal fills an IOSurface-backed MTLTexture and signals an MTLSharedEvent;
//   Vulkan (MoltenVK) imports BOTH via VK_EXT_metal_objects
//     - IOSurface -> VkImage  (VkImportMetalIOSurfaceInfoEXT)
//     - MTLSharedEvent -> timeline VkSemaphore (VkImportMetalSharedEventInfoEXT)
//   Vulkan waits on the semaphore, copies the image to a host-visible buffer,
//   and verifies the pixels match the checkerboard Metal wrote.
//
// Modes:
//   single                  0a: everything in one process (Metal write -> Vulkan read, cross-API sync)
//   create <id-file>        0b process N: create IOSurface (global), Metal-fill checkerboard, write IOSurfaceID to file, hold
//   import <id-file>        0b process W: IOSurfaceLookup(id) -> import as VkImage -> read back -> verify
//
// Build: see build_native.sh   (clang++ -ObjC++, links Metal/IOSurface/Foundation + libvulkan)
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <IOSurface/IOSurface.h>
#include <vulkan/vulkan.h>
#include <vulkan/vulkan_metal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <vector>

static const uint32_t W = 64, H = 64;          // surface dims
static const uint32_t BPP = 4;                  // BGRA8
static const VkFormat VKFMT = VK_FORMAT_B8G8R8A8_UNORM;

#define VK_CHECK(x) do { VkResult _r = (x); if (_r != VK_SUCCESS) { \
    fprintf(stderr, "FATAL %s:%d  %s = %d\n", __FILE__, __LINE__, #x, _r); exit(2);} } while(0)

// ----- the known pattern: a BGRA checkerboard, deterministic per (x,y) -----
static void fill_checker(uint8_t* p, uint32_t w, uint32_t h, uint32_t stride) {
    for (uint32_t y = 0; y < h; y++) {
        uint8_t* row = p + y * stride;
        for (uint32_t x = 0; x < w; x++) {
            bool on = ((x >> 3) ^ (y >> 3)) & 1;     // 8x8 squares
            uint8_t* px = row + x * BPP;
            px[0] = on ? 0xFF : 0x10;  // B
            px[1] = (uint8_t)(x * 4);  // G gradient (position-encoded)
            px[2] = (uint8_t)(y * 4);  // R gradient
            px[3] = 0xFF;              // A
        }
    }
}

// ============================ Vulkan setup ============================
struct Vk {
    VkInstance inst;
    VkPhysicalDevice phys;
    VkDevice dev;
    VkQueue queue;
    uint32_t qfam;
    PFN_vkExportMetalObjectsEXT pExportMetalObjects;
};

static Vk vk_init() {
    Vk v{};
    const char* inst_exts[] = {
        "VK_KHR_portability_enumeration",
        "VK_KHR_get_physical_device_properties2",
        "VK_KHR_external_memory_capabilities",
        "VK_KHR_external_semaphore_capabilities",
    };
    VkApplicationInfo ai{VK_STRUCTURE_TYPE_APPLICATION_INFO};
    ai.apiVersion = VK_API_VERSION_1_2;
    VkInstanceCreateInfo ici{VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO};
    ici.flags = 0x00000001; // VK_INSTANCE_CREATE_ENUMERATE_PORTABILITY_BIT_KHR
    ici.pApplicationInfo = &ai;
    ici.enabledExtensionCount = 4;
    ici.ppEnabledExtensionNames = inst_exts;
    // With the loader present, portability-enumeration is needed. When linking MoltenVK
    // directly (the x86_64 build vs CrossOver's ICD), those instance exts/flag don't exist
    // -> retry bare so the same source serves both paths.
    VkResult ir = vkCreateInstance(&ici, nullptr, &v.inst);
    if (ir != VK_SUCCESS) {
        fprintf(stderr, "[vk] portability-enum instance create = %d; retrying bare\n", ir);
        ici.flags = 0; ici.enabledExtensionCount = 0; ici.ppEnabledExtensionNames = nullptr;
        ir = vkCreateInstance(&ici, nullptr, &v.inst);
    }
    if (ir != VK_SUCCESS) { fprintf(stderr, "FATAL vkCreateInstance=%d\n", ir); exit(2); }

    uint32_t n = 0; VK_CHECK(vkEnumeratePhysicalDevices(v.inst, &n, nullptr));
    std::vector<VkPhysicalDevice> devs(n);
    VK_CHECK(vkEnumeratePhysicalDevices(v.inst, &n, devs.data()));
    v.phys = devs[0];
    VkPhysicalDeviceProperties pp; vkGetPhysicalDeviceProperties(v.phys, &pp);
    fprintf(stderr, "[vk] device: %s (api %u.%u.%u)\n", pp.deviceName,
            VK_VERSION_MAJOR(pp.apiVersion), VK_VERSION_MINOR(pp.apiVersion), VK_VERSION_PATCH(pp.apiVersion));

    // confirm metal_objects is present
    uint32_t ec = 0; vkEnumerateDeviceExtensionProperties(v.phys, nullptr, &ec, nullptr);
    std::vector<VkExtensionProperties> de(ec);
    vkEnumerateDeviceExtensionProperties(v.phys, nullptr, &ec, de.data());
    bool has_mo = false, has_portsub = false;
    for (auto& e : de) {
        if (!strcmp(e.extensionName, "VK_EXT_metal_objects")) has_mo = true;
        if (!strcmp(e.extensionName, "VK_KHR_portability_subset")) has_portsub = true;
    }
    fprintf(stderr, "[vk] VK_EXT_metal_objects advertised by device: %s\n", has_mo ? "YES" : "NO");
    if (!has_mo) { fprintf(stderr, "FATAL: device does not expose VK_EXT_metal_objects\n"); exit(3); }

    // queue
    uint32_t qn = 0; vkGetPhysicalDeviceQueueFamilyProperties(v.phys, &qn, nullptr);
    std::vector<VkQueueFamilyProperties> qfp(qn);
    vkGetPhysicalDeviceQueueFamilyProperties(v.phys, &qn, qfp.data());
    v.qfam = 0;
    for (uint32_t i = 0; i < qn; i++) if (qfp[i].queueFlags & VK_QUEUE_GRAPHICS_BIT) { v.qfam = i; break; }

    std::vector<const char*> dev_exts = {
        "VK_EXT_metal_objects",
        "VK_KHR_external_memory",
        "VK_KHR_external_semaphore",
        "VK_KHR_timeline_semaphore",
    };
    if (has_portsub) dev_exts.push_back("VK_KHR_portability_subset");

    VkPhysicalDeviceTimelineSemaphoreFeatures tsf{VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES};
    tsf.timelineSemaphore = VK_TRUE;
    float prio = 1.0f;
    VkDeviceQueueCreateInfo qci{VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO};
    qci.queueFamilyIndex = v.qfam; qci.queueCount = 1; qci.pQueuePriorities = &prio;
    VkDeviceCreateInfo dci{VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO};
    dci.pNext = &tsf;
    dci.queueCreateInfoCount = 1; dci.pQueueCreateInfos = &qci;
    dci.enabledExtensionCount = (uint32_t)dev_exts.size();
    dci.ppEnabledExtensionNames = dev_exts.data();
    VK_CHECK(vkCreateDevice(v.phys, &dci, nullptr, &v.dev));
    vkGetDeviceQueue(v.dev, v.qfam, 0, &v.queue);

    v.pExportMetalObjects = (PFN_vkExportMetalObjectsEXT)vkGetDeviceProcAddr(v.dev, "vkExportMetalObjectsEXT");
    fprintf(stderr, "[vk] vkExportMetalObjectsEXT resolved: %s\n", v.pExportMetalObjects ? "YES" : "NO");
    return v;
}

// Pull MoltenVK's MTLDevice so Metal-side allocations match the Vulkan device.
static id<MTLDevice> vk_get_mtldevice(Vk& v) {
    VkExportMetalDeviceInfoEXT di{VK_STRUCTURE_TYPE_EXPORT_METAL_DEVICE_INFO_EXT};
    VkExportMetalObjectsInfoEXT oi{VK_STRUCTURE_TYPE_EXPORT_METAL_OBJECTS_INFO_EXT};
    oi.pNext = &di;
    v.pExportMetalObjects(v.dev, &oi);
    return (__bridge id<MTLDevice>)di.mtlDevice;
}

// import an IOSurface as a sampleable/transfer-src VkImage
static VkImage vk_import_iosurface(Vk& v, IOSurfaceRef surf) {
    VkImportMetalIOSurfaceInfoEXT io{VK_STRUCTURE_TYPE_IMPORT_METAL_IO_SURFACE_INFO_EXT};
    io.ioSurface = surf;
    VkExternalMemoryImageCreateInfo ext{VK_STRUCTURE_TYPE_EXTERNAL_MEMORY_IMAGE_CREATE_INFO};
    ext.pNext = &io;
    ext.handleTypes = 0; // metal_objects import is signalled purely by the pNext struct
    VkImageCreateInfo ici{VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
    ici.pNext = &io;     // chain the IOSurface import directly
    ici.imageType = VK_IMAGE_TYPE_2D;
    ici.format = VKFMT;
    ici.extent = {W, H, 1};
    ici.mipLevels = 1; ici.arrayLayers = 1;
    ici.samples = VK_SAMPLE_COUNT_1_BIT;
    ici.tiling = VK_IMAGE_TILING_OPTIMAL;
    ici.usage = VK_IMAGE_USAGE_TRANSFER_SRC_BIT | VK_IMAGE_USAGE_SAMPLED_BIT;
    ici.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    ici.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
    VkImage img = VK_NULL_HANDLE;
    VK_CHECK(vkCreateImage(v.dev, &ici, nullptr, &img));
    // MoltenVK backs metal-imported images itself; bind memory only if it asks for it.
    VkMemoryRequirements mr{}; vkGetImageMemoryRequirements(v.dev, img, &mr);
    fprintf(stderr, "[vk] imported-image memory req size=%llu (0 => self-backed)\n", (unsigned long long)mr.size);
    if (mr.size > 0) {
        VkMemoryAllocateInfo mai{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
        mai.allocationSize = mr.size;
        // pick any memory type in the mask
        VkPhysicalDeviceMemoryProperties mp; vkGetPhysicalDeviceMemoryProperties(v.phys, &mp);
        for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
            if (mr.memoryTypeBits & (1u << i)) { mai.memoryTypeIndex = i; break; }
        VkDeviceMemory mem;
        VK_CHECK(vkAllocateMemory(v.dev, &mai, nullptr, &mem));
        VK_CHECK(vkBindImageMemory(v.dev, img, mem, 0));
    }
    return img;
}

static VkSemaphore vk_import_event(Vk& v, id<MTLSharedEvent> ev) {
    VkImportMetalSharedEventInfoEXT ie{VK_STRUCTURE_TYPE_IMPORT_METAL_SHARED_EVENT_INFO_EXT};
    ie.mtlSharedEvent = (__bridge MTLSharedEvent_id)ev;
    VkSemaphoreTypeCreateInfo tc{VK_STRUCTURE_TYPE_SEMAPHORE_TYPE_CREATE_INFO};
    tc.pNext = &ie;
    tc.semaphoreType = VK_SEMAPHORE_TYPE_TIMELINE;
    tc.initialValue = 0;
    VkSemaphoreCreateInfo sci{VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO};
    sci.pNext = &tc;
    VkSemaphore sem = VK_NULL_HANDLE;
    VK_CHECK(vkCreateSemaphore(v.dev, &sci, nullptr, &sem));
    return sem;
}

// copy imported image -> host buffer, optionally waiting on a timeline semaphore at waitVal.
// returns mapped bytes (W*H*BPP) in `out`.
static void vk_readback(Vk& v, VkImage img, VkSemaphore waitSem, uint64_t waitVal, std::vector<uint8_t>& out) {
    VkDeviceSize sz = (VkDeviceSize)W * H * BPP;
    VkBufferCreateInfo bci{VK_STRUCTURE_TYPE_BUFFER_CREATE_INFO};
    bci.size = sz; bci.usage = VK_BUFFER_USAGE_TRANSFER_DST_BIT; bci.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
    VkBuffer buf; VK_CHECK(vkCreateBuffer(v.dev, &bci, nullptr, &buf));
    VkMemoryRequirements mr; vkGetBufferMemoryRequirements(v.dev, buf, &mr);
    VkPhysicalDeviceMemoryProperties mp; vkGetPhysicalDeviceMemoryProperties(v.phys, &mp);
    uint32_t mti = 0;
    for (uint32_t i = 0; i < mp.memoryTypeCount; i++)
        if ((mr.memoryTypeBits & (1u << i)) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_VISIBLE_BIT) &&
            (mp.memoryTypes[i].propertyFlags & VK_MEMORY_PROPERTY_HOST_COHERENT_BIT)) { mti = i; break; }
    VkMemoryAllocateInfo mai{VK_STRUCTURE_TYPE_MEMORY_ALLOCATE_INFO};
    mai.allocationSize = mr.size; mai.memoryTypeIndex = mti;
    VkDeviceMemory bmem; VK_CHECK(vkAllocateMemory(v.dev, &mai, nullptr, &bmem));
    VK_CHECK(vkBindBufferMemory(v.dev, buf, bmem, 0));

    VkCommandPoolCreateInfo pci{VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO};
    pci.queueFamilyIndex = v.qfam;
    VkCommandPool pool; VK_CHECK(vkCreateCommandPool(v.dev, &pci, nullptr, &pool));
    VkCommandBufferAllocateInfo cbi{VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO};
    cbi.commandPool = pool; cbi.level = VK_COMMAND_BUFFER_LEVEL_PRIMARY; cbi.commandBufferCount = 1;
    VkCommandBuffer cmd; VK_CHECK(vkAllocateCommandBuffers(v.dev, &cbi, &cmd));

    VkCommandBufferBeginInfo bi{VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    bi.flags = VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT;
    VK_CHECK(vkBeginCommandBuffer(cmd, &bi));
    VkImageMemoryBarrier b{VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER};
    b.oldLayout = VK_IMAGE_LAYOUT_UNDEFINED; b.newLayout = VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
    b.srcAccessMask = 0; b.dstAccessMask = VK_ACCESS_TRANSFER_READ_BIT;
    b.image = img; b.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    b.srcQueueFamilyIndex = b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    vkCmdPipelineBarrier(cmd, VK_PIPELINE_STAGE_TOP_OF_PIPE_BIT, VK_PIPELINE_STAGE_TRANSFER_BIT, 0, 0, nullptr, 0, nullptr, 1, &b);
    VkBufferImageCopy region{};
    region.imageSubresource = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 0, 1};
    region.imageExtent = {W, H, 1};
    vkCmdCopyImageToBuffer(cmd, img, VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL, buf, 1, &region);
    VK_CHECK(vkEndCommandBuffer(cmd));

    VkTimelineSemaphoreSubmitInfo ts{VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO};
    uint64_t wv = waitVal;
    VkPipelineStageFlags waitStage = VK_PIPELINE_STAGE_TRANSFER_BIT;
    VkSubmitInfo si{VK_STRUCTURE_TYPE_SUBMIT_INFO};
    si.commandBufferCount = 1; si.pCommandBuffers = &cmd;
    if (waitSem) {
        ts.waitSemaphoreValueCount = 1; ts.pWaitSemaphoreValues = &wv;
        si.pNext = &ts;
        si.waitSemaphoreCount = 1; si.pWaitSemaphores = &waitSem; si.pWaitDstStageMask = &waitStage;
    }
    VK_CHECK(vkQueueSubmit(v.queue, 1, &si, VK_NULL_HANDLE));
    VK_CHECK(vkQueueWaitIdle(v.queue));

    void* mapped = nullptr;
    VK_CHECK(vkMapMemory(v.dev, bmem, 0, sz, 0, &mapped));
    out.resize(sz);
    memcpy(out.data(), mapped, sz);
    vkUnmapMemory(v.dev, bmem);
}

// ============================ Metal helpers ============================
static IOSurfaceRef make_iosurface(bool global) {
    NSMutableDictionary* props = [@{
        (id)kIOSurfaceWidth: @(W),
        (id)kIOSurfaceHeight: @(H),
        (id)kIOSurfaceBytesPerElement: @(BPP),
        (id)kIOSurfacePixelFormat: @((uint32_t)'BGRA'),
    } mutableCopy];
    if (global) props[@"IOSurfaceIsGlobal"] = @YES;  // legacy: make ID-lookupable cross-process
    IOSurfaceRef s = IOSurfaceCreate((CFDictionaryRef)props);
    if (!s) { fprintf(stderr, "FATAL IOSurfaceCreate failed\n"); exit(4); }
    return s;
}

// Metal-fill the IOSurface-backed texture with the checkerboard, signalling `ev` to 1 on GPU completion.
static void metal_fill(id<MTLDevice> dev, IOSurfaceRef surf, id<MTLSharedEvent> ev) {
    MTLTextureDescriptor* td = [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm
                                                                                 width:W height:H mipmapped:NO];
    td.usage = MTLTextureUsageShaderRead | MTLTextureUsageShaderWrite;
    td.storageMode = MTLStorageModeShared;
    id<MTLTexture> tex = [dev newTextureWithDescriptor:td iosurface:surf plane:0];
    if (!tex) { fprintf(stderr, "FATAL newTextureWithDescriptor:iosurface failed\n"); exit(5); }

    std::vector<uint8_t> checker(W * H * BPP);
    fill_checker(checker.data(), W, H, W * BPP);
    // write straight into the texture (shared storage => visible to GPU/IOSurface)
    [tex replaceRegion:MTLRegionMake2D(0, 0, W, H) mipmapLevel:0
             withBytes:checker.data() bytesPerRow:W * BPP];

    id<MTLCommandQueue> q = [dev newCommandQueue];
    id<MTLCommandBuffer> cb = [q commandBuffer];
    // a trivial blit so there is real GPU work to order the signal after
    id<MTLBlitCommandEncoder> blit = [cb blitCommandEncoder];
    [blit synchronizeResource:tex];
    [blit endEncoding];
    if (ev) [cb encodeSignalEvent:ev value:1];
    [cb commit];
    [cb waitUntilCompleted];
    fprintf(stderr, "[metal] filled IOSurface + signalled event to 1\n");
}

static int compare(const std::vector<uint8_t>& got) {
    std::vector<uint8_t> want(W * H * BPP);
    fill_checker(want.data(), W, H, W * BPP);
    size_t mism = 0; size_t firstbad = SIZE_MAX;
    for (size_t i = 0; i < want.size(); i++)
        if (want[i] != got[i]) { if (firstbad == SIZE_MAX) firstbad = i; mism++; }
    fprintf(stderr, "[verify] bytes=%zu mismatched=%zu\n", want.size(), mism);
    if (mism) {
        size_t i = firstbad;
        fprintf(stderr, "[verify] first mismatch at byte %zu (pixel %zu): want=%02X got=%02X\n",
                i, i / BPP, want[i], got[i]);
        fprintf(stderr, "[verify] sample want[0..7]: %02X %02X %02X %02X %02X %02X %02X %02X\n",
                want[0],want[1],want[2],want[3],want[4],want[5],want[6],want[7]);
        fprintf(stderr, "[verify] sample  got[0..7]: %02X %02X %02X %02X %02X %02X %02X %02X\n",
                got[0],got[1],got[2],got[3],got[4],got[5],got[6],got[7]);
    }
    return mism == 0 ? 0 : 1;
}

// ============================ modes ============================
static int mode_single() {
    @autoreleasepool {
        fprintf(stderr, "=== Gate 0a: single-process IOSurface+MTLSharedEvent through MoltenVK ===\n");
        Vk v = vk_init();
        id<MTLDevice> mdev = vk_get_mtldevice(v);
        fprintf(stderr, "[metal] MoltenVK MTLDevice: %s\n", mdev ? mdev.name.UTF8String : "(null)");
        if (!mdev) { fprintf(stderr, "FATAL could not export MoltenVK MTLDevice\n"); return 6; }

        IOSurfaceRef surf = make_iosurface(false);
        id<MTLSharedEvent> ev = [mdev newSharedEvent];
        metal_fill(mdev, surf, ev);

        VkImage img = vk_import_iosurface(v, surf);
        VkSemaphore sem = vk_import_event(v, ev);
        std::vector<uint8_t> got;
        vk_readback(v, img, sem, 1, got);
        int rc = compare(got);
        fprintf(stdout, "GATE0A_RESULT: %s\n", rc == 0 ? "PASS" : "FAIL");
        return rc;
    }
}

static int mode_create(const char* idfile) {
    @autoreleasepool {
        fprintf(stderr, "=== Gate 0b process N: create global IOSurface + Metal fill ===\n");
        // use MoltenVK's device so the surface/texture are on the same GPU the importer will use
        Vk v = vk_init();
        id<MTLDevice> mdev = vk_get_mtldevice(v);
        IOSurfaceRef surf = make_iosurface(true);
        IOSurfaceID sid = IOSurfaceGetID(surf);
        metal_fill(mdev, surf, nil); // sync object sharing handled separately; prove pixels first
        FILE* f = fopen(idfile, "w"); fprintf(f, "%u\n", (unsigned)sid); fclose(f);
        fprintf(stderr, "[N] IOSurfaceID=%u written to %s; holding 60s\n", (unsigned)sid, idfile);
        fflush(stderr);
        [NSThread sleepForTimeInterval:60.0];
        return 0;
    }
}

static int mode_import(const char* idfile) {
    @autoreleasepool {
        fprintf(stderr, "=== Gate 0b process W: lookup IOSurface by ID + Vulkan import + verify ===\n");
        FILE* f = fopen(idfile, "r"); if (!f) { fprintf(stderr, "FATAL no id file %s\n", idfile); return 7; }
        unsigned sid = 0; if (fscanf(f, "%u", &sid) != 1) { fprintf(stderr, "FATAL bad id file\n"); return 7; } fclose(f);
        fprintf(stderr, "[W] looking up IOSurfaceID=%u\n", sid);
        IOSurfaceRef surf = IOSurfaceLookup((IOSurfaceID)sid);
        if (!surf) { fprintf(stderr, "FATAL IOSurfaceLookup(%u) returned NULL (cross-proc lookup failed)\n", sid); return 8; }
        fprintf(stderr, "[W] looked up surface %ux%u\n", (unsigned)IOSurfaceGetWidth(surf), (unsigned)IOSurfaceGetHeight(surf));
        Vk v = vk_init();
        VkImage img = vk_import_iosurface(v, surf);
        std::vector<uint8_t> got;
        vk_readback(v, img, VK_NULL_HANDLE, 0, got);
        int rc = compare(got);
        fprintf(stdout, "GATE0B_RESULT: %s\n", rc == 0 ? "PASS" : "FAIL");
        return rc;
    }
}

int main(int argc, char** argv) {
    const char* mode = argc > 1 ? argv[1] : "single";
    if (!strcmp(mode, "single")) return mode_single();
    if (!strcmp(mode, "create")) return mode_create(argc > 2 ? argv[2] : "/tmp/gate0_surfid.txt");
    if (!strcmp(mode, "import")) return mode_import(argc > 2 ? argv[2] : "/tmp/gate0_surfid.txt");
    fprintf(stderr, "usage: %s single|create <idfile>|import <idfile>\n", argv[0]);
    return 1;
}
