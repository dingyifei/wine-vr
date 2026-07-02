// oxrsys_cubes.mm — minimal native macOS OpenXR client (Metal binding) for Gate 1.
//
// Renders a set of world-locked colored cubes per eye and submits a projection layer
// every frame so oxrsys composites + encodes + streams it to the Quest. Proves the
// runtime works end-to-end on this machine: if the cubes appear head-locked (i.e. stay
// put in the world as you turn your head), the OpenXR pose path + streaming both work.
//
// Binding: XR_KHR_metal_enable (oxrsys' most-exercised native path; the plan allows a
// Metal-binding client for Gate 1; the eventual Wine app uses Vulkan in Gate 3).
//
// Build: see build_cubes.sh
#import <Foundation/Foundation.h>
#import <Metal/Metal.h>
#import <QuartzCore/QuartzCore.h>
#include <simd/simd.h>
#define XR_USE_GRAPHICS_API_METAL 1
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <stdio.h>
#include <vector>
#include <cmath>

#define XRC(x) do { XrResult _r=(x); if(XR_FAILED(_r)){ fprintf(stderr,"XR FAIL %s:%d %s = %d\n",__FILE__,__LINE__,#x,_r); exit(2);} } while(0)

// ---------- math ----------
static simd_float4x4 proj_fov(const XrFovf& fov, float n, float f) {
    float tl=tanf(fov.angleLeft), tr=tanf(fov.angleRight), tu=tanf(fov.angleUp), td=tanf(fov.angleDown);
    float w=tr-tl, h=tu-td;
    simd_float4x4 m = {};
    m.columns[0] = (simd_float4){2.0f/w, 0, 0, 0};
    m.columns[1] = (simd_float4){0, 2.0f/h, 0, 0};
    m.columns[2] = (simd_float4){(tr+tl)/w, (tu+td)/h, -(f)/(f-n), -1};
    m.columns[3] = (simd_float4){0, 0, -(f*n)/(f-n), 0};
    return m;
}
static simd_float4x4 mat_from_pose(const XrPosef& p) {
    // rotation from quaternion
    simd_quatf q = simd_quaternion(p.orientation.x, p.orientation.y, p.orientation.z, p.orientation.w);
    simd_float4x4 r = simd_matrix4x4(q);
    r.columns[3] = (simd_float4){p.position.x, p.position.y, p.position.z, 1};
    return r;
}
static simd_float4x4 translate_scale(simd_float3 t, float s) {
    simd_float4x4 m = matrix_identity_float4x4;
    m.columns[0].x = s; m.columns[1].y = s; m.columns[2].z = s;
    m.columns[3] = (simd_float4){t.x, t.y, t.z, 1};
    return m;
}

// ---------- cube geometry (pos + per-face color via normal) ----------
struct Vtx { simd_float3 pos; simd_float3 col; };
static std::vector<Vtx> cube_mesh(simd_float3 color) {
    // 12 triangles
    simd_float3 c = color;
    auto C=[&](float x,float y,float z){ return (Vtx){{x,y,z}, c}; };
    std::vector<Vtx> v;
    float s=0.5f;
    simd_float3 p[8] = {{-s,-s,-s},{s,-s,-s},{s,s,-s},{-s,s,-s},{-s,-s,s},{s,-s,s},{s,s,s},{-s,s,s}};
    int faces[6][4] = {{0,1,2,3},{5,4,7,6},{4,0,3,7},{1,5,6,2},{3,2,6,7},{4,5,1,0}};
    float shade[6] = {0.55f,0.65f,0.75f,0.85f,1.0f,0.45f};
    for (int f=0; f<6; f++){
        simd_float3 fc = c*shade[f];
        Vtx a={p[faces[f][0]],fc},b={p[faces[f][1]],fc},d={p[faces[f][2]],fc},e={p[faces[f][3]],fc};
        v.push_back(a); v.push_back(b); v.push_back(d);
        v.push_back(a); v.push_back(d); v.push_back(e);
    }
    return v;
}

static const char* kShader = R"(
#include <metal_stdlib>
using namespace metal;
struct Vtx { float3 pos; float3 col; };
struct VOut { float4 pos [[position]]; float3 col; };
struct Uniforms { float4x4 mvp; };
vertex VOut vmain(uint vid [[vertex_id]],
                  const device Vtx* verts [[buffer(0)]],
                  constant Uniforms& u [[buffer(1)]]) {
    VOut o;
    o.pos = u.mvp * float4(verts[vid].pos, 1.0);
    o.col = verts[vid].col;
    return o;
}
fragment float4 fmain(VOut in [[stage_in]]) { return float4(in.col, 1.0); }
)";

int main(int argc, char** argv) {
@autoreleasepool {
    int maxFrames = argc > 1 ? atoi(argv[1]) : 100000;
    FILE* tcsv = fopen("evidence/gate1-timing.csv", "w");
    if (tcsv) fprintf(tcsv, "frame,app_render_ms,submit_ms\n");

    // ---- instance ----
    const char* exts[] = { XR_KHR_METAL_ENABLE_EXTENSION_NAME };
    XrInstanceCreateInfo ici{XR_TYPE_INSTANCE_CREATE_INFO};
    strcpy(ici.applicationInfo.applicationName, "oxrsys_cubes");
    ici.applicationInfo.apiVersion = XR_CURRENT_API_VERSION;
    ici.enabledExtensionCount = 1; ici.enabledExtensionNames = exts;
    XrInstance instance; XRC(xrCreateInstance(&ici, &instance));
    XrInstanceProperties ip{XR_TYPE_INSTANCE_PROPERTIES};
    xrGetInstanceProperties(instance, &ip);
    fprintf(stderr, "[xr] runtime: %s (v%llu.%llu.%llu)\n", ip.runtimeName,
            XR_VERSION_MAJOR(ip.runtimeVersion), XR_VERSION_MINOR(ip.runtimeVersion), XR_VERSION_PATCH(ip.runtimeVersion));

    XrSystemGetInfo sgi{XR_TYPE_SYSTEM_GET_INFO};
    sgi.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId sys; XRC(xrGetSystem(instance, &sgi, &sys));

    PFN_xrGetMetalGraphicsRequirementsKHR getReq=nullptr;
    XRC(xrGetInstanceProcAddr(instance, "xrGetMetalGraphicsRequirementsKHR", (PFN_xrVoidFunction*)&getReq));
    XrGraphicsRequirementsMetalKHR req{XR_TYPE_GRAPHICS_REQUIREMENTS_METAL_KHR};
    XRC(getReq(instance, sys, &req));
    id<MTLDevice> dev = (__bridge id<MTLDevice>)req.metalDevice;
    if (!dev) dev = MTLCreateSystemDefaultDevice();
    fprintf(stderr, "[metal] device: %s\n", dev.name.UTF8String);
    id<MTLCommandQueue> queue = [dev newCommandQueue];

    // ---- session ----
    XrGraphicsBindingMetalKHR gb{XR_TYPE_GRAPHICS_BINDING_METAL_KHR};
    gb.commandQueue = (__bridge void*)queue;
    XrSessionCreateInfo sci{XR_TYPE_SESSION_CREATE_INFO};
    sci.next = &gb; sci.systemId = sys;
    XrSession session; XRC(xrCreateSession(instance, &sci, &session));

    // ---- spaces ----
    XrReferenceSpaceCreateInfo rsci{XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
    rsci.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    rsci.poseInReferenceSpace.orientation.w = 1;
    XrSpace appSpace; XRC(xrCreateReferenceSpace(session, &rsci, &appSpace));

    // ---- view config ----
    uint32_t viewCount=0;
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &viewCount, nullptr));
    std::vector<XrViewConfigurationView> cfgViews(viewCount, {XR_TYPE_VIEW_CONFIGURATION_VIEW});
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, viewCount, &viewCount, cfgViews.data()));
    fprintf(stderr, "[xr] %u views, %ux%u\n", viewCount, cfgViews[0].recommendedImageRectWidth, cfgViews[0].recommendedImageRectHeight);

    // ---- swapchain format ----
    uint32_t fmtCount=0; XRC(xrEnumerateSwapchainFormats(session,0,&fmtCount,nullptr));
    std::vector<int64_t> fmts(fmtCount); XRC(xrEnumerateSwapchainFormats(session,fmtCount,&fmtCount,fmts.data()));
    int64_t colorFormat = fmts[0];
    for (int64_t f : fmts) if (f==MTLPixelFormatBGRA8Unorm_sRGB || f==MTLPixelFormatBGRA8Unorm){colorFormat=f;break;}
    fprintf(stderr, "[xr] swapchain format=%lld\n", (long long)colorFormat);

    // ---- per-eye swapchains ----
    struct Eye { XrSwapchain sc; uint32_t w,h; std::vector<id<MTLTexture>> tex; std::vector<id<MTLTexture>> depth; };
    std::vector<Eye> eyes(viewCount);
    for (uint32_t i=0;i<viewCount;i++){
        XrSwapchainCreateInfo ci{XR_TYPE_SWAPCHAIN_CREATE_INFO};
        ci.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_SAMPLED_BIT;
        ci.format = colorFormat; ci.sampleCount = 1;
        ci.width = cfgViews[i].recommendedImageRectWidth;
        ci.height = cfgViews[i].recommendedImageRectHeight;
        ci.faceCount=1; ci.arraySize=1; ci.mipCount=1;
        XRC(xrCreateSwapchain(session, &ci, &eyes[i].sc));
        eyes[i].w=ci.width; eyes[i].h=ci.height;
        uint32_t imgCount=0; XRC(xrEnumerateSwapchainImages(eyes[i].sc,0,&imgCount,nullptr));
        std::vector<XrSwapchainImageMetalKHR> imgs(imgCount, {XR_TYPE_SWAPCHAIN_IMAGE_METAL_KHR});
        XRC(xrEnumerateSwapchainImages(eyes[i].sc, imgCount, &imgCount, (XrSwapchainImageBaseHeader*)imgs.data()));
        for (auto& im : imgs) eyes[i].tex.push_back((__bridge id<MTLTexture>)im.texture);
        // depth textures (private) per swapchain image
        for (uint32_t k=0;k<imgCount;k++){
            MTLTextureDescriptor* dd=[MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatDepth32Float width:eyes[i].w height:eyes[i].h mipmapped:NO];
            dd.usage=MTLTextureUsageRenderTarget; dd.storageMode=MTLStorageModePrivate;
            eyes[i].depth.push_back([dev newTextureWithDescriptor:dd]);
        }
        fprintf(stderr, "[xr] eye %u swapchain %ux%u images=%u\n", i, eyes[i].w, eyes[i].h, imgCount);
    }

    // ---- pipeline ----
    NSError* err=nil;
    id<MTLLibrary> lib=[dev newLibraryWithSource:[NSString stringWithUTF8String:kShader] options:nil error:&err];
    if(!lib){ fprintf(stderr,"shader err: %s\n", err.localizedDescription.UTF8String); return 3; }
    MTLRenderPipelineDescriptor* pd=[MTLRenderPipelineDescriptor new];
    pd.vertexFunction=[lib newFunctionWithName:@"vmain"];
    pd.fragmentFunction=[lib newFunctionWithName:@"fmain"];
    pd.colorAttachments[0].pixelFormat=(MTLPixelFormat)colorFormat;
    pd.depthAttachmentPixelFormat=MTLPixelFormatDepth32Float;
    id<MTLRenderPipelineState> pso=[dev newRenderPipelineStateWithDescriptor:pd error:&err];
    if(!pso){ fprintf(stderr,"pso err: %s\n", err.localizedDescription.UTF8String); return 3; }
    MTLDepthStencilDescriptor* dsd=[MTLDepthStencilDescriptor new];
    dsd.depthCompareFunction=MTLCompareFunctionLess; dsd.depthWriteEnabled=YES;
    id<MTLDepthStencilState> dss=[dev newDepthStencilStateWithDescriptor:dsd];

    // ---- scene: ring of cubes + floor grid of cubes ----
    struct Cube { simd_float3 pos; float scale; simd_float3 color; };
    std::vector<Cube> cubes;
    for (int i=0;i<8;i++){ float a=i*(M_PI*2/8); cubes.push_back({{2.0f*cosf(a),0,-2.0f*sinf(a)},0.3f,{(float)(i&1),(float)((i>>1)&1),(float)((i>>2)&1)}}); }
    for (int x=-2;x<=2;x++) for(int z=-2;z<=2;z++) cubes.push_back({{(float)x, -1.0f, (float)z-1.0f}, 0.15f, {0.2f,0.8f,0.3f}});
    // one cube straight ahead, close, distinctive
    cubes.push_back({{0,0,-1.0f}, 0.25f, {1.0f,0.2f,0.2f}});

    std::vector<Vtx> mesh = cube_mesh((simd_float3){1,1,1});
    id<MTLBuffer> vbuf=[dev newBufferWithBytes:mesh.data() length:mesh.size()*sizeof(Vtx) options:MTLResourceStorageModeShared];
    uint32_t vcount=(uint32_t)mesh.size();

    // ---- frame loop ----
    bool running=true, sessionRunning=false; XrSessionState state=XR_SESSION_STATE_UNKNOWN;
    int frame=0;
    while (running && frame<maxFrames) {
        XrEventDataBuffer ev{XR_TYPE_EVENT_DATA_BUFFER};
        while (xrPollEvent(instance, &ev)==XR_SUCCESS) {
            if (ev.type==XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
                auto* ss=(XrEventDataSessionStateChanged*)&ev;
                state=ss->state;
                fprintf(stderr,"[xr] session state -> %d\n", state);
                if (state==XR_SESSION_STATE_READY){
                    XrSessionBeginInfo bi{XR_TYPE_SESSION_BEGIN_INFO};
                    bi.primaryViewConfigurationType=XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                    XRC(xrBeginSession(session,&bi)); sessionRunning=true;
                } else if (state==XR_SESSION_STATE_STOPPING){ XRC(xrEndSession(session)); sessionRunning=false; }
                else if (state==XR_SESSION_STATE_EXITING||state==XR_SESSION_STATE_LOSS_PENDING){ running=false; }
            }
            ev = {XR_TYPE_EVENT_DATA_BUFFER};
        }
        if (!sessionRunning){ usleep(10000); continue; }

        XrFrameState fs{XR_TYPE_FRAME_STATE};
        XRC(xrWaitFrame(session, nullptr, &fs));
        XRC(xrBeginFrame(session, nullptr));

        std::vector<XrCompositionLayerProjectionView> projViews(viewCount, {XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW});
        XrCompositionLayerProjection layer{XR_TYPE_COMPOSITION_LAYER_PROJECTION};
        bool gotViews=false;

        if (fs.shouldRender) {
            XrViewState vs{XR_TYPE_VIEW_STATE};
            uint32_t vc=viewCount; std::vector<XrView> views(viewCount,{XR_TYPE_VIEW});
            XrViewLocateInfo vli{XR_TYPE_VIEW_LOCATE_INFO};
            vli.viewConfigurationType=XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
            vli.displayTime=fs.predictedDisplayTime; vli.space=appSpace;
            XRC(xrLocateViews(session,&vli,&vs,viewCount,&vc,views.data()));

            double t_render0 = CACurrentMediaTime();
            for (uint32_t e=0;e<viewCount;e++){
                uint32_t idx=0; XRC(xrAcquireSwapchainImage(eyes[e].sc,nullptr,&idx));
                XrSwapchainImageWaitInfo wi{XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO}; wi.timeout=XR_INFINITE_DURATION;
                XRC(xrWaitSwapchainImage(eyes[e].sc,&wi));

                simd_float4x4 P=proj_fov(views[e].fov,0.05f,100.0f);
                simd_float4x4 V=simd_inverse(mat_from_pose(views[e].pose));
                simd_float4x4 VP=simd_mul(P,V);

                MTLRenderPassDescriptor* rp=[MTLRenderPassDescriptor new];
                rp.colorAttachments[0].texture=eyes[e].tex[idx];
                rp.colorAttachments[0].loadAction=MTLLoadActionClear;
                rp.colorAttachments[0].storeAction=MTLStoreActionStore;
                rp.colorAttachments[0].clearColor=MTLClearColorMake(0.05,0.05,0.1,1.0);
                rp.depthAttachment.texture=eyes[e].depth[idx];
                rp.depthAttachment.loadAction=MTLLoadActionClear; rp.depthAttachment.clearDepth=1.0;
                rp.depthAttachment.storeAction=MTLStoreActionDontCare;

                id<MTLCommandBuffer> cb=[queue commandBuffer];
                id<MTLRenderCommandEncoder> enc=[cb renderCommandEncoderWithDescriptor:rp];
                [enc setRenderPipelineState:pso];
                [enc setDepthStencilState:dss];
                [enc setVertexBuffer:vbuf offset:0 atIndex:0];
                for (auto& c : cubes){
                    simd_float4x4 M=translate_scale(c.pos,c.scale);
                    simd_float4x4 mvp=simd_mul(VP,M);
                    struct { simd_float4x4 mvp; } u={mvp};
                    [enc setVertexBytes:&u length:sizeof(u) atIndex:1];
                    [enc drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:vcount];
                }
                [enc endEncoding];
                [cb commit];
                [cb waitUntilCompleted];

                XrSwapchainImageReleaseInfo ri{XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
                XRC(xrReleaseSwapchainImage(eyes[e].sc,&ri));

                projViews[e].pose=views[e].pose; projViews[e].fov=views[e].fov;
                projViews[e].subImage.swapchain=eyes[e].sc;
                projViews[e].subImage.imageRect.offset={0,0};
                projViews[e].subImage.imageRect.extent={(int32_t)eyes[e].w,(int32_t)eyes[e].h};
            }
            double t_render1 = CACurrentMediaTime();
            layer.space=appSpace; layer.viewCount=viewCount; layer.views=projViews.data();
            gotViews=true;
            if (tcsv && frame<2000) fprintf(tcsv, "%d,%.3f,\n", frame, (t_render1-t_render0)*1000.0);
        }

        const XrCompositionLayerBaseHeader* layers[1]={(XrCompositionLayerBaseHeader*)&layer};
        XrFrameEndInfo fei{XR_TYPE_FRAME_END_INFO};
        fei.displayTime=fs.predictedDisplayTime;
        fei.environmentBlendMode=XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount=gotViews?1:0; fei.layers=gotViews?layers:nullptr;
        double t_s0=CACurrentMediaTime();
        XRC(xrEndFrame(session,&fei));
        double t_s1=CACurrentMediaTime();
        if (tcsv && gotViews && frame<2000){ fseek(tcsv,0,SEEK_END); }
        if (frame%90==0) fprintf(stderr,"[frame %d] shouldRender=%d state=%d submit=%.2fms\n", frame, fs.shouldRender, state, (t_s1-t_s0)*1000.0);
        frame++;
    }
    if (tcsv) fclose(tcsv);
    fprintf(stderr,"[done] frames=%d\n", frame);
    xrDestroySession(session);
    xrDestroyInstance(instance);
    return 0;
}
}
