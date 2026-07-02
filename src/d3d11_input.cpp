// d3d11_input.cpp — controller-input derisk for the wineopenxr -> oxrsys bridge.
//
// Extends d3d11_clear with the full OpenXR action system: an action set with a
// hand-pose action and a trigger/select boolean, suggested bindings for the
// simple + Meta Touch-plus interaction profiles, per-frame xrSyncActions, and
// xrLocateSpace for the controller poses. This proves Quest controllers reach a
// PE OpenXR app through the bridge — the prerequisite for any real game.
//
// Live feedback in the headset: each eye is red/blue normally, and turns bright
// GREEN while that hand's trigger is held, and brightness tracks controller
// height. Pull a trigger -> that eye flashes green. Controller state is also
// logged (every 90 frames, and immediately on any trigger up/down edge).
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
#include <string>
#include <vector>
#include <cmath>

#define XRC(x) do { XrResult _r=(x); if(XR_FAILED(_r)){ printf("XR FAIL %s:%d %s = %d\n",__FILE__,__LINE__,#x,_r); fflush(stdout); return 2;} } while(0)
#define HRC(x) do { HRESULT _h=(x); if(FAILED(_h)){ printf("HR FAIL %s:%d %s = 0x%08lx\n",__FILE__,__LINE__,#x,(unsigned long)_h); fflush(stdout); return 3;} } while(0)

int main(int argc, char** argv) {
    int maxFrames = argc > 1 ? atoi(argv[1]) : 100000;
    freopen("C:\\d3d11_log.txt", "w", stdout);
    freopen("C:\\d3d11_log.txt", "a", stderr);
    setvbuf(stdout, nullptr, _IONBF, 0);

    // ---- instance ----
    const char* exts[] = { XR_KHR_D3D11_ENABLE_EXTENSION_NAME };
    XrInstanceCreateInfo ici{XR_TYPE_INSTANCE_CREATE_INFO};
    strcpy(ici.applicationInfo.applicationName, "d3d11_input");
    ici.applicationInfo.apiVersion = XR_MAKE_VERSION(1, 1, 0);
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

    // ---- input action system ----
    auto strp = [&](const char* s){ XrPath p=XR_NULL_PATH; xrStringToPath(instance,s,&p); return p; };

    XrActionSet actionSet;
    { XrActionSetCreateInfo asci{XR_TYPE_ACTION_SET_CREATE_INFO};
      strcpy(asci.actionSetName,"gameplay"); strcpy(asci.localizedActionSetName,"Gameplay");
      XRC(xrCreateActionSet(instance,&asci,&actionSet)); }

    XrPath handPath[2] = { strp("/user/hand/left"), strp("/user/hand/right") };

    XrAction poseAction, selectAction;
    { XrActionCreateInfo aci{XR_TYPE_ACTION_CREATE_INFO}; aci.actionType=XR_ACTION_TYPE_POSE_INPUT;
      strcpy(aci.actionName,"hand_pose"); strcpy(aci.localizedActionName,"Hand Pose");
      aci.countSubactionPaths=2; aci.subactionPaths=handPath;
      XRC(xrCreateAction(actionSet,&aci,&poseAction)); }
    { XrActionCreateInfo aci{XR_TYPE_ACTION_CREATE_INFO}; aci.actionType=XR_ACTION_TYPE_BOOLEAN_INPUT;
      strcpy(aci.actionName,"select"); strcpy(aci.localizedActionName,"Select/Trigger");
      aci.countSubactionPaths=2; aci.subactionPaths=handPath;
      XRC(xrCreateAction(actionSet,&aci,&selectAction)); }

    // Baseline: the KHR simple controller is mandatory for every conformant runtime and
    // maps select/click onto the Quest trigger. Also suggest Meta Touch-plus (what oxrsys
    // advertised) for the real trigger. A rejected suggestion is logged, not fatal.
    auto suggest = [&](const char* profile, const char* selPath){
        XrActionSuggestedBinding b[] = {
            { poseAction,   strp("/user/hand/left/input/aim/pose") },
            { poseAction,   strp("/user/hand/right/input/aim/pose") },
            { selectAction, strp((std::string("/user/hand/left")  + selPath).c_str()) },
            { selectAction, strp((std::string("/user/hand/right") + selPath).c_str()) },
        };
        XrInteractionProfileSuggestedBinding s{XR_TYPE_INTERACTION_PROFILE_SUGGESTED_BINDING};
        s.interactionProfile=strp(profile); s.suggestedBindings=b; s.countSuggestedBindings=4;
        XrResult r=xrSuggestInteractionProfileBindings(instance,&s);
        printf("[input] suggest %s = %d\n", profile, r);
    };
    suggest("/interaction_profiles/khr/simple_controller", "/input/select/click");
    suggest("/interaction_profiles/meta/touch_plus_controller", "/input/trigger/value");
    suggest("/interaction_profiles/oculus/touch_controller", "/input/trigger/value");

    XrSpace handSpace[2];
    for (int h=0; h<2; h++) {
        XrActionSpaceCreateInfo si{XR_TYPE_ACTION_SPACE_CREATE_INFO};
        si.action=poseAction; si.subactionPath=handPath[h]; si.poseInActionSpace.orientation.w=1;
        XRC(xrCreateActionSpace(session,&si,&handSpace[h]));
    }
    { XrSessionActionSetsAttachInfo ai{XR_TYPE_SESSION_ACTION_SETS_ATTACH_INFO};
      ai.countActionSets=1; ai.actionSets=&actionSet;
      XRC(xrAttachSessionActionSets(session,&ai)); }
    printf("[input] action set attached\n");

    uint32_t viewCount=0;
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, 0, &viewCount, nullptr));
    std::vector<XrViewConfigurationView> cfgViews(viewCount, {XR_TYPE_VIEW_CONFIGURATION_VIEW});
    XRC(xrEnumerateViewConfigurationViews(instance, sys, XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO, viewCount, &viewCount, cfgViews.data()));

    uint32_t fmtCount=0; XRC(xrEnumerateSwapchainFormats(session,0,&fmtCount,nullptr));
    std::vector<int64_t> fmts(fmtCount); XRC(xrEnumerateSwapchainFormats(session,fmtCount,&fmtCount,fmts.data()));
    int64_t colorFormat = fmts[0];
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
    int frame=0; uint64_t clearsOk=0; bool prevSel[2]={false,false};
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
            } else if (ev.type==XR_TYPE_EVENT_DATA_INTERACTION_PROFILE_CHANGED) {
                XrInteractionProfileState ips{XR_TYPE_INTERACTION_PROFILE_STATE};
                if (XR_SUCCEEDED(xrGetCurrentInteractionProfile(session,handPath[0],&ips))) {
                    char buf[XR_MAX_PATH_LENGTH]; uint32_t len=0;
                    if (ips.interactionProfile!=XR_NULL_PATH &&
                        XR_SUCCEEDED(xrPathToString(instance,ips.interactionProfile,sizeof(buf),&len,buf)))
                        printf("[input] active interaction profile (left) = %s\n", buf);
                    else printf("[input] interaction profile changed (left) = none\n");
                }
            }
            ev = {XR_TYPE_EVENT_DATA_BUFFER};
        }
        if (!sessionRunning){ Sleep(10); continue; }

        XrFrameState fs{XR_TYPE_FRAME_STATE};
        XRC(xrWaitFrame(session,nullptr,&fs));
        XRC(xrBeginFrame(session,nullptr));

        // ---- input: sync + read controllers (only meaningful while focused) ----
        bool selDown[2]={false,false}, handValid[2]={false,false};
        XrVector3f handPos[2]={};
        if (state==XR_SESSION_STATE_FOCUSED) {
            XrActiveActionSet aas{actionSet, XR_NULL_PATH};
            XrActionsSyncInfo syncInfo{XR_TYPE_ACTIONS_SYNC_INFO};
            syncInfo.countActiveActionSets=1; syncInfo.activeActionSets=&aas;
            xrSyncActions(session,&syncInfo);   // XR_SESSION_NOT_FOCUSED is a benign qualified-success
            for (int h=0; h<2; h++) {
                XrActionStateGetInfo gi{XR_TYPE_ACTION_STATE_GET_INFO};
                gi.action=selectAction; gi.subactionPath=handPath[h];
                XrActionStateBoolean bs{XR_TYPE_ACTION_STATE_BOOLEAN};
                if (XR_SUCCEEDED(xrGetActionStateBoolean(session,&gi,&bs)))
                    selDown[h] = bs.isActive && bs.currentState;

                XrActionStateGetInfo gp{XR_TYPE_ACTION_STATE_GET_INFO};
                gp.action=poseAction; gp.subactionPath=handPath[h];
                XrActionStatePose ps{XR_TYPE_ACTION_STATE_POSE};
                xrGetActionStatePose(session,&gp,&ps);
                if (ps.isActive) {
                    XrSpaceLocation loc{XR_TYPE_SPACE_LOCATION};
                    if (XR_SUCCEEDED(xrLocateSpace(handSpace[h],appSpace,fs.predictedDisplayTime,&loc)) &&
                        (loc.locationFlags & XR_SPACE_LOCATION_POSITION_VALID_BIT)) {
                        handPos[h]=loc.pose.position; handValid[h]=true;
                    }
                }
            }
            for (int h=0; h<2; h++) if (selDown[h]!=prevSel[h]) {
                printf("[input] %s TRIGGER %s pos=(%.2f,%.2f,%.2f) valid=%d\n",
                       h?"R":"L", selDown[h]?"DOWN":"up ",
                       handPos[h].x,handPos[h].y,handPos[h].z, handValid[h]);
                prevSel[h]=selDown[h];
            }
        }

        std::vector<XrCompositionLayerProjectionView> pv(viewCount,{XR_TYPE_COMPOSITION_LAYER_PROJECTION_VIEW});
        XrCompositionLayerProjection layer{XR_TYPE_COMPOSITION_LAYER_PROJECTION};
        bool didRender=false;
        if (fs.shouldRender) {
            XrViewState vs{XR_TYPE_VIEW_STATE}; uint32_t vc=viewCount;
            std::vector<XrView> views(viewCount,{XR_TYPE_VIEW});
            XrViewLocateInfo vli{XR_TYPE_VIEW_LOCATE_INFO};
            vli.viewConfigurationType=XR_VIEW_CONFIGURATION_TYPE_PRIMARY_STEREO;
            vli.displayTime=fs.predictedDisplayTime; vli.space=appSpace;
            XRC(xrLocateViews(session,&vli,&vs,viewCount,&vc,views.data()));

            float t = (float)(fs.predictedDisplayTime % 2000000000LL) / 1e9f;
            float pulse = 0.6f + 0.4f*sinf(t*3.0f);
            for (uint32_t e=0;e<viewCount;e++){
                uint32_t idx=0; XRC(xrAcquireSwapchainImage(eyes[e].sc,nullptr,&idx));
                XrSwapchainImageWaitInfo wi{XR_TYPE_SWAPCHAIN_IMAGE_WAIT_INFO}; wi.timeout=XR_INFINITE_DURATION;
                XRC(xrWaitSwapchainImage(eyes[e].sc,&wi));
                ID3D11RenderTargetView* rtv=nullptr;
                D3D11_RENDER_TARGET_VIEW_DESC rd{}; rd.Format=(DXGI_FORMAT)colorFormat;
                rd.ViewDimension=D3D11_RTV_DIMENSION_TEXTURE2D;
                HRESULT rhr = dev->CreateRenderTargetView(eyes[e].tex[idx], &rd, &rtv);
                if (FAILED(rhr)) { D3D11_TEXTURE2D_DESC td{}; eyes[e].tex[idx]->GetDesc(&td);
                    rd.Format = td.Format; rhr = dev->CreateRenderTargetView(eyes[e].tex[idx], &rd, &rtv); }
                if (SUCCEEDED(rhr) && rtv) {
                    // Trigger held on this hand -> that eye flashes bright green; otherwise
                    // red (left) / blue (right). Brightness tracks controller height so
                    // moving the controllers visibly changes the image.
                    bool trig = selDown[e];
                    float lift = handValid[e] ? (0.5f + 0.5f*fmaxf(0.f, fminf(1.f, handPos[e].y+0.5f))) : 1.0f;
                    float col[4]={0,0,0,1};
                    if (trig) { col[1]=0.9f*lift; }                       // green
                    else if (e==0) { col[0]=0.85f*pulse*lift; }           // red
                    else { col[2]=0.85f*pulse*lift; }                     // blue
                    ctx->ClearRenderTargetView(rtv, col);
                    rtv->Release();
                    clearsOk++;
                }
                pv[e].pose=views[e].pose; pv[e].fov=views[e].fov;
                pv[e].subImage.swapchain=eyes[e].sc;
                pv[e].subImage.imageRect.offset={0,0};
                pv[e].subImage.imageRect.extent={(int32_t)eyes[e].w,(int32_t)eyes[e].h};
            }
            ctx->Flush();
            for (uint32_t e=0;e<viewCount;e++){
                XrSwapchainImageReleaseInfo ri{XR_TYPE_SWAPCHAIN_IMAGE_RELEASE_INFO};
                XRC(xrReleaseSwapchainImage(eyes[e].sc,&ri));
            }
            layer.space=appSpace; layer.viewCount=viewCount; layer.views=pv.data(); didRender=true;
        }
        const XrCompositionLayerBaseHeader* layers[1]={(XrCompositionLayerBaseHeader*)&layer};
        XrFrameEndInfo fei{XR_TYPE_FRAME_END_INFO};
        fei.displayTime=fs.predictedDisplayTime; fei.environmentBlendMode=XR_ENVIRONMENT_BLEND_MODE_OPAQUE;
        fei.layerCount=didRender?1:0; fei.layers=didRender?layers:nullptr;
        XRC(xrEndFrame(session,&fei));
        if (frame%90==0) printf("[frame %d] state=%d L{sel=%d pos=(%.2f,%.2f,%.2f) v=%d} R{sel=%d pos=(%.2f,%.2f,%.2f) v=%d}\n",
                                frame, state,
                                selDown[0],handPos[0].x,handPos[0].y,handPos[0].z,handValid[0],
                                selDown[1],handPos[1].x,handPos[1].y,handPos[1].z,handValid[1]);
        frame++;
    }
    printf("[done] frames=%d clears=%llu\n", frame, (unsigned long long)clearsOk);
    xrDestroySession(session); xrDestroyInstance(instance);
    return 0;
}
