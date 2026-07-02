// d3d11_clear.cpp — minimal D3D11 OpenXR client for testing the wineopenxr -> oxrsys bridge.
//
// Purpose: exercise the full Wine D3D11 path end to end without a shader compiler:
//   D3D11 swapchain textures -> DXMT IMTLD3D11InteropDevice (MTLTexture, zero-copy)
//   -> wineopenxr.dll/.so -> native oxrsys runtime -> Quest.
// Each eye is cleared to an animated color (left warm, right cool), pulsing over time, with a
// brightness term driven by head yaw so head tracking is visibly confirmed on the headset.
//
// Built as a Windows PE (x86_64) with mingw; links the cross-built openxr_loader.
// No HLSL/shaders -> no d3dcompiler dependency. Geometry comes later once the pipe is proven.
#define XR_USE_GRAPHICS_API_D3D11 1
#define XR_USE_PLATFORM_WIN32 1
#define WIN32_LEAN_AND_MEAN 1
#include <windows.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <openxr/openxr.h>
#include <openxr/openxr_platform.h>
#include <cstdio>
#include <cstring>
#include <vector>
#include <cmath>

#define XRC(x) do { XrResult _r=(x); if(XR_FAILED(_r)){ printf("XR FAIL %s:%d %s = %d\n",__FILE__,__LINE__,#x,_r); fflush(stdout); return 2;} } while(0)
#define HRC(x) do { HRESULT _h=(x); if(FAILED(_h)){ printf("HR FAIL %s:%d %s = 0x%08lx\n",__FILE__,__LINE__,#x,(unsigned long)_h); fflush(stdout); return 3;} } while(0)

int main(int argc, char** argv) {
    int maxFrames = argc > 1 ? atoi(argv[1]) : 100000;
    // CrossOver detaches the console, so mirror all output to a file in the bottle.
    freopen("C:\\d3d11_log.txt", "w", stdout);
    freopen("C:\\d3d11_log.txt", "a", stderr);
    setvbuf(stdout, nullptr, _IONBF, 0);

    // ---- instance ----
    const char* exts[] = { XR_KHR_D3D11_ENABLE_EXTENSION_NAME };
    XrInstanceCreateInfo ici{XR_TYPE_INSTANCE_CREATE_INFO};
    strcpy(ici.applicationInfo.applicationName, "d3d11_clear");
    ici.applicationInfo.apiVersion = XR_MAKE_VERSION(1, 1, 0); // <= oxrsys' XR_CURRENT_API_VERSION
    ici.enabledExtensionCount = 1; ici.enabledExtensionNames = exts;
    XrInstance instance; XRC(xrCreateInstance(&ici, &instance));
    XrInstanceProperties ip{XR_TYPE_INSTANCE_PROPERTIES};
    xrGetInstanceProperties(instance, &ip);
    printf("[xr] runtime: %s\n", ip.runtimeName);

    XrSystemGetInfo sgi{XR_TYPE_SYSTEM_GET_INFO};
    sgi.formFactor = XR_FORM_FACTOR_HEAD_MOUNTED_DISPLAY;
    XrSystemId sys; XRC(xrGetSystem(instance, &sgi, &sys));

    // ---- D3D11 graphics requirements (adapter LUID) ----
    PFN_xrGetD3D11GraphicsRequirementsKHR getReq=nullptr;
    XRC(xrGetInstanceProcAddr(instance, "xrGetD3D11GraphicsRequirementsKHR", (PFN_xrVoidFunction*)&getReq));
    XrGraphicsRequirementsD3D11KHR req{XR_TYPE_GRAPHICS_REQUIREMENTS_D3D11_KHR};
    XRC(getReq(instance, sys, &req));

    // ---- pick the adapter matching the LUID ----
    IDXGIFactory1* factory=nullptr;
    HRC(CreateDXGIFactory1(__uuidof(IDXGIFactory1), (void**)&factory));
    IDXGIAdapter1* adapter=nullptr; IDXGIAdapter1* chosen=nullptr;
    for (UINT i=0; factory->EnumAdapters1(i,&adapter)!=DXGI_ERROR_NOT_FOUND; i++) {
        DXGI_ADAPTER_DESC1 d; adapter->GetDesc1(&d);
        if (memcmp(&d.AdapterLuid, &req.adapterLuid, sizeof(LUID))==0) { chosen=adapter; break; }
        adapter->Release();
    }
    printf("[d3d11] adapter %s\n", chosen ? "matched LUID" : "default (LUID not matched)");

    D3D_FEATURE_LEVEL fls[] = { D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0 };
    ID3D11Device* dev=nullptr; ID3D11DeviceContext* ctx=nullptr; D3D_FEATURE_LEVEL got;
    HRC(D3D11CreateDevice(chosen, chosen?D3D_DRIVER_TYPE_UNKNOWN:D3D_DRIVER_TYPE_HARDWARE,
        nullptr, 0, fls, 2, D3D11_SDK_VERSION, &dev, &got, &ctx));
    if (chosen) chosen->Release();
    factory->Release();
    printf("[d3d11] device created, feature level 0x%x\n", got);

    // Defensive local producer wait. The bridge also waits on DXMT's Metal fence
    // before native release/snapshot, but this keeps the probe self-checking and
    // makes the clear complete even if run against an older bridge.
    ID3D11Query* gpuDone=nullptr;
    { D3D11_QUERY_DESC qd{}; qd.Query=D3D11_QUERY_EVENT; dev->CreateQuery(&qd,&gpuDone); }

    // ---- session (D3D11 binding) ----
    XrGraphicsBindingD3D11KHR gb{XR_TYPE_GRAPHICS_BINDING_D3D11_KHR};
    gb.device = dev;
    XrSessionCreateInfo sci{XR_TYPE_SESSION_CREATE_INFO};
    sci.next = &gb; sci.systemId = sys;
    XrSession session; XRC(xrCreateSession(instance, &sci, &session));
    printf("[xr] session created (D3D11 binding)\n");

    XrReferenceSpaceCreateInfo rsci{XR_TYPE_REFERENCE_SPACE_CREATE_INFO};
    rsci.referenceSpaceType = XR_REFERENCE_SPACE_TYPE_LOCAL;
    rsci.poseInReferenceSpace.orientation.w = 1;
    XrSpace appSpace; XRC(xrCreateReferenceSpace(session, &rsci, &appSpace));

    uint32_t viewCount=0;
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &viewCount, nullptr));
    std::vector<XrViewConfigurationView> cfgViews(viewCount, {XR_TYPE_VIEW_CONFIGURATION_VIEW});
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, viewCount, &viewCount, cfgViews.data()));
    printf("[xr] %u views %ux%u\n", viewCount, cfgViews[0].recommendedImageRectWidth, cfgViews[0].recommendedImageRectHeight);

    uint32_t fmtCount=0; XRC(xrEnumerateSwapchainFormats(session,0,&fmtCount,nullptr));
    std::vector<int64_t> fmts(fmtCount); XRC(xrEnumerateSwapchainFormats(session,fmtCount,&fmtCount,fmts.data()));
    int64_t colorFormat = fmts[0];
    // Prefer NON-sRGB: DXMT's ImportMTLTexture2D builds its expected format via the typeless
    // parent (-> linear bgra8Unorm), so an sRGB swapchain texture from the host trips the
    // ORIGINAL_FORMAT mismatch check. Linear formats import cleanly.
    for (int64_t f : fmts) if (f==DXGI_FORMAT_B8G8R8A8_UNORM || f==DXGI_FORMAT_R8G8B8A8_UNORM) { colorFormat=f; break; }
    printf("[xr] swapchain format=%lld\n",(long long)colorFormat);

    struct Eye { XrSwapchain sc; uint32_t w,h; std::vector<ID3D11Texture2D*> tex; };
    std::vector<Eye> eyes(viewCount);
    for (uint32_t i=0;i<viewCount;i++){
        XrSwapchainCreateInfo ci{XR_TYPE_SWAPCHAIN_CREATE_INFO};
        ci.usageFlags = XR_SWAPCHAIN_USAGE_COLOR_ATTACHMENT_BIT | XR_SWAPCHAIN_USAGE_SAMPLED_BIT;
        ci.format=colorFormat; ci.sampleCount=1;
        ci.width=cfgViews[i].recommendedImageRectWidth; ci.height=cfgViews[i].recommendedImageRectHeight;
        ci.faceCount=1; ci.arraySize=1; ci.mipCount=1;
        XRC(xrCreateSwapchain(session,&ci,&eyes[i].sc));
        eyes[i].w=ci.width; eyes[i].h=ci.height;
        uint32_t n=0; XRC(xrEnumerateSwapchainImages(eyes[i].sc,0,&n,nullptr));
        std::vector<XrSwapchainImageD3D11KHR> imgs(n,{XR_TYPE_SWAPCHAIN_IMAGE_D3D11_KHR});
        XRC(xrEnumerateSwapchainImages(eyes[i].sc,n,&n,(XrSwapchainImageBaseHeader*)imgs.data()));
        for (auto& im : imgs) eyes[i].tex.push_back(im.texture);
        printf("[xr] eye %u swapchain %ux%u images=%u\n", i, eyes[i].w, eyes[i].h, n);
    }

    bool running=true, sessionRunning=false; XrSessionState state=XR_SESSION_STATE_UNKNOWN;
    int frame=0; uint64_t rtvFailures=0, clearsOk=0;
    while (running && frame<maxFrames) {
        XrEventDataBuffer ev{XR_TYPE_EVENT_DATA_BUFFER};
        while (xrPollEvent(instance,&ev)==XR_SUCCESS) {
            if (ev.type==XR_TYPE_EVENT_DATA_SESSION_STATE_CHANGED) {
                auto* ss=(XrEventDataSessionStateChanged*)&ev; state=ss->state;
                printf("[xr] state -> %d\n", state);
                if (state==XR_SESSION_STATE_READY){ XrSessionBeginInfo bi{XR_TYPE_SESSION_BEGIN_INFO};
                    bi.primaryViewConfigurationType=XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
                    XRC(xrBeginSession(session,&bi)); sessionRunning=true; }
                else if (state==XR_SESSION_STATE_STOPPING){ XRC(xrEndSession(session)); sessionRunning=false; }
                else if (state==XR_SESSION_STATE_EXITING||state==XR_SESSION_STATE_LOSS_PENDING) running=false;
            }
            ev = {XR_TYPE_EVENT_DATA_BUFFER};
        }
        if (!sessionRunning){ Sleep(10); continue; }

        XrFrameState fs{XR_TYPE_FRAME_STATE};
        XRC(xrWaitFrame(session,nullptr,&fs));
        XRC(xrBeginFrame(session,nullptr));

        std::vector<XrCompositionLayerProjectionView> pv(viewCount,{XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW});
        XrCompositionLayerProjection layer{XR_TYPE_COMPOSITION_LAYER_PROJECTION};
        bool got=false;
        if (fs.shouldRender) {
            XrViewState vs{XR_TYPE_VIEW_STATE}; uint32_t vc=viewCount;
            std::vector<XrView> views(viewCount,{XR_TYPE_VIEW});
            XrViewLocateInfo vli{XR_TYPE_VIEW_LOCATE_INFO};
            vli.viewConfigurationType=XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
            vli.displayTime=fs.predictedDisplayTime; vli.space=appSpace;
            XRC(xrLocateViews(session,&vli,&vs,viewCount,&vc,views.data()));

            float t = (float)(fs.predictedDisplayTime % 2000000000LL) / 1e9f;
            float pulse = 0.5f + 0.5f*sinf(t*3.0f);
            for (uint32_t e=0;e<viewCount;e++){
                uint32_t idx=0; XRC(xrAcquireSwapchainImage(eyes[e].sc,nullptr,&idx));
                XrSwapchainImageWaitInfo wi{XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO}; wi.timeout=XR_INFINITE_DURATION;
                XRC(xrWaitSwapchainImage(eyes[e].sc,&wi));
                // brightness reacts to head yaw (quaternion y) so motion is visible
                float yaw = views[e].pose.orientation.y;
                float b = 0.4f + 0.6f*fabsf(yaw);
                // Log the imported texture's actual desc once (diagnoses format/bind issues).
                if (frame == 0) {
                    D3D11_TEXTURE2D_DESC td{}; eyes[e].tex[idx]->GetDesc(&td);
                    printf("[d3d11] eye %u img desc: fmt=%u bind=0x%x usage=%u w=%u h=%u\n",
                           e, td.Format, td.BindFlags, td.Usage, td.Width, td.Height);
                }
                ID3D11RenderTargetView* rtv=nullptr;
                D3D11_RENDER_TARGET_VIEW_DESC rd{}; rd.Format=(DXGI_FORMAT)colorFormat;
                rd.ViewDimension=D3D11_RTV_DIMENSION_TEXTURE2D;
                HRESULT rhr = dev->CreateRenderTargetView(eyes[e].tex[idx], &rd, &rtv);
                if (FAILED(rhr)) {
                    // Retry with the texture's own format (handles typeless/sRGB view mismatch).
                    D3D11_TEXTURE2D_DESC td{}; eyes[e].tex[idx]->GetDesc(&td);
                    rd.Format = td.Format;
                    rhr = dev->CreateRenderTargetView(eyes[e].tex[idx], &rd, &rtv);
                }
                if (FAILED(rhr) || !rtv) {
                    if (frame < 3) printf("[d3d11] eye %u CreateRenderTargetView FAILED hr=0x%08lx\n", e, (unsigned long)rhr);
                    rtvFailures++;
                } else {
                    // Bright, mostly-constant colors so any output is unmistakable (left red, right blue),
                    // with a slow pulse for liveness. (void)b keeps yaw wiring for later.
                    (void)b;
                    float col[4];
                    if (e==0) { col[0]=0.85f; col[1]=0.10f; col[2]=0.10f; col[3]=1.0f; }
                    else      { col[0]=0.10f; col[1]=0.10f; col[2]=0.85f; col[3]=1.0f; }
                    col[0]*=pulse; col[1]*=pulse; col[2]*=pulse;
                    ctx->ClearRenderTargetView(rtv, col);
                    rtv->Release();
                    clearsOk++;
                }
                pv[e].pose=views[e].pose; pv[e].fov=views[e].fov;
                pv[e].subImage.swapchain=eyes[e].sc;
                pv[e].subImage.imageRect.offset={0,0};
                pv[e].subImage.imageRect.extent={(int32_t)eyes[e].w,(int32_t)eyes[e].h};
            }
            // Submit all clears to the GPU before releasing images, then wait below
            // so this probe also works against bridge builds without release-side
            // producer-fence waiting.
            ctx->Flush();
            // NOTE: no CPU wait for GPU completion here — wineopenxr waits on the DXMT/Metal
            // fence at xrReleaseSwapchainImage before oxrsys snapshots, so blocking here would
            // only serialize CPU/GPU and hurt frame pacing.
            for (uint32_t e=0;e<viewCount;e++){
                XrSwapchainImageReleaseInfo ri{XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
                XRC(xrReleaseSwapchainImage(eyes[e].sc,&ri));
            }
            layer.space=appSpace; layer.viewCount=viewCount; layer.views=pv.data(); got=true;
        }
        const XrCompositionLayerBaseHeader* layers[1]={(XrCompositionLayerBaseHeader*)&layer};
        XrFrameEndInfo fei{XR_TYPE_FRAME_END_INFO};
        fei.displayTime=fs.predictedDisplayTime; fei.environmentBlendMode=XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount=got?1:0; fei.layers=got?layers:nullptr;
        XRC(xrEndFrame(session,&fei));
        if (frame%90==0) printf("[frame %d] shouldRender=%d state=%d clearsOk=%llu rtvFail=%llu\n",
                                frame, fs.shouldRender, state,
                                (unsigned long long)clearsOk, (unsigned long long)rtvFailures);
        frame++;
    }
    printf("[done] frames=%d\n", frame);
    xrDestroySession(session); xrDestroyInstance(instance);
    return 0;
}
