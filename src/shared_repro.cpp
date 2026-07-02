// shared_repro.cpp — minimal D3D11 cross-process-shareable resource probe.
//
// Reproduces the exact capability SteamVR's vrcompositor needs at init:
//   creating D3D11 resources with D3D11_RESOURCE_MISC_SHARED / SHARED_NTHANDLE /
//   SHARED_KEYEDMUTEX, exporting an NT handle, and re-opening it from ANOTHER process.
//
// Modes:
//   (no arg)      single-process probe of buffer + texture, SHARED/NTHANDLE/KEYEDMUTEX
//   create-tex    create a 1024x1024 BGRA NTHANDLE|KEYEDMUTEX texture, export named handle, hold ~25s
//   open-tex      open the named texture handle (run in a 2nd process while create-tex holds)
//   create-buf    same for a 256-byte constant buffer
//   open-buf      open the named buffer handle
//
// Build (macOS, mingw-w64): see build.sh.  Run in bottle with CX_GRAPHICS_BACKEND set.

#define INITGUID
#include <windows.h>
#include <d3d11_1.h>
#include <dxgi1_2.h>
#include <stdio.h>

static const wchar_t* TEX_NAME = L"Local\\wine_vr_repro_tex";
static const wchar_t* BUF_NAME = L"Local\\wine_vr_repro_cb";

static const char* hrname(HRESULT hr) {
    switch ((unsigned)hr) {
        case 0x00000000: return "S_OK";
        case 0x80070057: return "E_INVALIDARG";
        case 0x80004001: return "E_NOTIMPL";
        case 0x80004002: return "E_NOINTERFACE";
        case 0x8007000E: return "E_OUTOFMEMORY";
        case 0x887A0001: return "DXGI_ERROR_INVALID_CALL";
        case 0x887A0004: return "DXGI_ERROR_UNSUPPORTED";
        case 0x887A0005: return "DXGI_ERROR_DEVICE_REMOVED";
        case 0x80070032: return "ERROR_NOT_SUPPORTED";
        default: return "?";
    }
}
#define SHOW(label, hr) printf("  %-46s hr=0x%08X (%s)\n", label, (unsigned)(hr), hrname(hr))

static ID3D11Device* make_device(ID3D11Device1** outDev1) {
    D3D_FEATURE_LEVEL got = (D3D_FEATURE_LEVEL)0;
    ID3D11Device *dev = NULL; ID3D11DeviceContext *ctx = NULL;
    HRESULT hr = D3D11CreateDevice(NULL, D3D_DRIVER_TYPE_HARDWARE, NULL,
                                   D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                                   NULL, 0, D3D11_SDK_VERSION, &dev, &got, &ctx);
    SHOW("D3D11CreateDevice (HARDWARE)", hr);
    if (FAILED(hr) || !dev) return NULL;
    printf("  feature level = 0x%04X\n", (unsigned)got);
    if (outDev1) { dev->QueryInterface(__uuidof(ID3D11Device1), (void**)outDev1); }
    if (ctx) ctx->Release();
    return dev;
}

static void test_keyed_mutex(IUnknown* res, const char* tag) {
    IDXGIKeyedMutex *km = NULL;
    HRESULT hr = res->QueryInterface(__uuidof(IDXGIKeyedMutex), (void**)&km);
    char b[64]; snprintf(b, sizeof b, "QI IDXGIKeyedMutex (%s)", tag); SHOW(b, hr);
    if (SUCCEEDED(hr) && km) {
        hr = km->AcquireSync(0, 100); snprintf(b, sizeof b, "  AcquireSync(0) (%s)", tag); SHOW(b, hr);
        hr = km->ReleaseSync(0);      snprintf(b, sizeof b, "  ReleaseSync(0) (%s)", tag); SHOW(b, hr);
        km->Release();
    }
}

static int cross_process(const char* mode) {
    ID3D11Device1 *dev1 = NULL;
    ID3D11Device *dev = make_device(&dev1);
    if (!dev) { printf("FATAL: no device\n"); return 2; }
    bool tex = (strstr(mode, "tex") != NULL);
    bool create = (strstr(mode, "create") != NULL);
    const wchar_t* name = tex ? TEX_NAME : BUF_NAME;

    if (create) {
        IDXGIResource1 *res = NULL; HRESULT hr;
        if (tex) {
            D3D11_TEXTURE2D_DESC td = {};
            td.Width = 1024; td.Height = 1024; td.MipLevels = 1; td.ArraySize = 1;
            td.Format = DXGI_FORMAT_B8G8R8A8_UNORM; td.SampleDesc.Count = 1;
            td.Usage = D3D11_USAGE_DEFAULT;
            td.BindFlags = D3D11_BIND_SHADER_RESOURCE | D3D11_BIND_RENDER_TARGET;
            td.MiscFlags = D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX;
            ID3D11Texture2D *t = NULL;
            hr = dev->CreateTexture2D(&td, NULL, &t); SHOW("CreateTexture2D(NTHANDLE|KEYEDMUTEX)", hr);
            if (FAILED(hr)) return 3;
            test_keyed_mutex(t, "src-tex");
            t->QueryInterface(__uuidof(IDXGIResource1), (void**)&res);
        } else {
            D3D11_BUFFER_DESC bd = {};
            bd.ByteWidth = 256; bd.Usage = D3D11_USAGE_DEFAULT;
            bd.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
            bd.MiscFlags = D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
            ID3D11Buffer *b = NULL;
            hr = dev->CreateBuffer(&bd, NULL, &b); SHOW("CreateBuffer(SHARED_NTHANDLE)", hr);
            if (FAILED(hr)) return 3;
            b->QueryInterface(__uuidof(IDXGIResource1), (void**)&res);
        }
        if (!res) { printf("FATAL: no IDXGIResource1\n"); return 4; }
        HANDLE h = NULL;
        hr = res->CreateSharedHandle(NULL, DXGI_SHARED_RESOURCE_READ|DXGI_SHARED_RESOURCE_WRITE, name, &h);
        SHOW("CreateSharedHandle(named)", hr);
        if (FAILED(hr)) return 5;
        printf("  EXPORTED named handle; holding open 25s for opener process...\n"); fflush(stdout);
        Sleep(25000);
        CloseHandle(h);
    } else {
        // opener — OpenSharedResourceByName returns the resource directly
        if (!dev1) { printf("FATAL: no ID3D11Device1\n"); return 6; }
        HRESULT hr;
        DWORD acc = DXGI_SHARED_RESOURCE_READ | DXGI_SHARED_RESOURCE_WRITE;
        if (tex) {
            ID3D11Texture2D *t = NULL;
            hr = dev1->OpenSharedResourceByName(name, acc, __uuidof(ID3D11Texture2D), (void**)&t);
            SHOW("OpenSharedResourceByName(tex)", hr);
            if (SUCCEEDED(hr) && t) { test_keyed_mutex(t, "opened-tex"); t->Release(); }
            else printf("  (could not open texture by name across process)\n");
        } else {
            ID3D11Buffer *b = NULL;
            hr = dev1->OpenSharedResourceByName(name, acc, __uuidof(ID3D11Buffer), (void**)&b);
            SHOW("OpenSharedResourceByName(buf)", hr);
            if (b) b->Release();
            else printf("  (could not open buffer by name across process)\n");
        }
    }
    if (dev1) dev1->Release();
    dev->Release();
    return 0;
}

int main(int argc, char** argv) {
    if (argc >= 2) {
        printf("=== shared_repro: cross-process mode '%s' ===\n", argv[1]);
        return cross_process(argv[1]);
    }
    printf("=== shared_repro: D3D11 shared-resource capability probe (single process) ===\n");
    ID3D11Device1 *dev1 = NULL;
    ID3D11Device *dev = make_device(&dev1);
    if (!dev) { printf("FATAL: no device\n"); return 2; }
    SHOW("QueryInterface ID3D11Device1", dev1 ? S_OK : E_NOINTERFACE);

    // [1] legacy MISC_SHARED constant buffer + legacy GetSharedHandle + OpenSharedResource
    printf("[1] constant buffer, D3D11_RESOURCE_MISC_SHARED (legacy non-NT path)\n");
    { D3D11_BUFFER_DESC bd = {}; bd.ByteWidth=256; bd.Usage=D3D11_USAGE_DEFAULT;
      bd.BindFlags=D3D11_BIND_CONSTANT_BUFFER; bd.MiscFlags=D3D11_RESOURCE_MISC_SHARED;
      ID3D11Buffer *b=NULL; HRESULT hr=dev->CreateBuffer(&bd,NULL,&b);
      SHOW("CreateBuffer(MISC_SHARED)", hr);
      if (SUCCEEDED(hr)&&b){ IDXGIResource *res=NULL; hr=b->QueryInterface(__uuidof(IDXGIResource),(void**)&res);
        SHOW("QI IDXGIResource", hr);
        if (SUCCEEDED(hr)&&res){ HANDLE h=NULL; hr=res->GetSharedHandle(&h);
          SHOW("GetSharedHandle (legacy)", hr);
          if (SUCCEEDED(hr)&&h){ ID3D11Buffer*o=NULL; hr=dev->OpenSharedResource(h,__uuidof(ID3D11Buffer),(void**)&o);
            SHOW("OpenSharedResource (legacy)", hr); if(o)o->Release(); } res->Release(); } b->Release(); } }

    // [2] SHARED_NTHANDLE constant buffer + export + same-process open
    printf("[2] constant buffer, SHARED_NTHANDLE  (SteamVR 'shared frame info constant buffer' analogue)\n");
    { D3D11_BUFFER_DESC bd = {}; bd.ByteWidth=256; bd.Usage=D3D11_USAGE_DEFAULT;
      bd.BindFlags=D3D11_BIND_CONSTANT_BUFFER; bd.MiscFlags=D3D11_RESOURCE_MISC_SHARED_NTHANDLE;
      ID3D11Buffer *b=NULL; HRESULT hr=dev->CreateBuffer(&bd,NULL,&b);
      SHOW("CreateBuffer(SHARED_NTHANDLE)", hr);
      if (SUCCEEDED(hr)&&b){ IDXGIResource1 *res=NULL; hr=b->QueryInterface(__uuidof(IDXGIResource1),(void**)&res);
        SHOW("QI IDXGIResource1", hr);
        if (SUCCEEDED(hr)&&res){ HANDLE h=NULL;
          hr=res->CreateSharedHandle(NULL,DXGI_SHARED_RESOURCE_READ|DXGI_SHARED_RESOURCE_WRITE,BUF_NAME,&h);
          SHOW("CreateSharedHandle(named)", hr);
          if (SUCCEEDED(hr)&&h&&dev1){ ID3D11Buffer*o=NULL; hr=dev1->OpenSharedResource1(h,__uuidof(ID3D11Buffer),(void**)&o);
            SHOW("OpenSharedResource1", hr); if(o)o->Release(); CloseHandle(h);} res->Release(); } b->Release(); } }

    // [3] SHARED_NTHANDLE | KEYEDMUTEX constant buffer
    printf("[3] constant buffer, SHARED_NTHANDLE | KEYEDMUTEX\n");
    { D3D11_BUFFER_DESC bd = {}; bd.ByteWidth=256; bd.Usage=D3D11_USAGE_DEFAULT;
      bd.BindFlags=D3D11_BIND_CONSTANT_BUFFER;
      bd.MiscFlags=D3D11_RESOURCE_MISC_SHARED_NTHANDLE|D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX;
      ID3D11Buffer *b=NULL; HRESULT hr=dev->CreateBuffer(&bd,NULL,&b);
      SHOW("CreateBuffer(NTHANDLE|KEYEDMUTEX)", hr);
      if (SUCCEEDED(hr)&&b){ test_keyed_mutex(b,"buf"); b->Release(); } }

    // [4] 1024x1024 BGRA texture, SHARED_NTHANDLE | KEYEDMUTEX (eye-buffer analogue)
    printf("[4] texture 1024x1024 BGRA, SHARED_NTHANDLE | KEYEDMUTEX  (VR eye-buffer analogue)\n");
    { D3D11_TEXTURE2D_DESC td = {}; td.Width=1024; td.Height=1024; td.MipLevels=1; td.ArraySize=1;
      td.Format=DXGI_FORMAT_B8G8R8A8_UNORM; td.SampleDesc.Count=1; td.Usage=D3D11_USAGE_DEFAULT;
      td.BindFlags=D3D11_BIND_SHADER_RESOURCE|D3D11_BIND_RENDER_TARGET;
      td.MiscFlags=D3D11_RESOURCE_MISC_SHARED_NTHANDLE|D3D11_RESOURCE_MISC_SHARED_KEYEDMUTEX;
      ID3D11Texture2D *t=NULL; HRESULT hr=dev->CreateTexture2D(&td,NULL,&t);
      SHOW("CreateTexture2D(NTHANDLE|KEYEDMUTEX)", hr);
      if (SUCCEEDED(hr)&&t){ test_keyed_mutex(t,"src-tex");
        IDXGIResource1 *res=NULL; hr=t->QueryInterface(__uuidof(IDXGIResource1),(void**)&res);
        SHOW("QI IDXGIResource1(tex)", hr);
        if (SUCCEEDED(hr)&&res){ HANDLE h=NULL;
          hr=res->CreateSharedHandle(NULL,DXGI_SHARED_RESOURCE_READ|DXGI_SHARED_RESOURCE_WRITE,TEX_NAME,&h);
          SHOW("CreateSharedHandle(tex,named)", hr);
          if (SUCCEEDED(hr)&&h&&dev1){ ID3D11Texture2D*o=NULL; hr=dev1->OpenSharedResource1(h,__uuidof(ID3D11Texture2D),(void**)&o);
            SHOW("OpenSharedResource1(tex)", hr);
            if (SUCCEEDED(hr)&&o){ test_keyed_mutex(o,"opened-tex"); o->Release(); } CloseHandle(h);} res->Release(); } t->Release(); } }

    printf("=== done ===\n");
    if (dev1) dev1->Release();
    dev->Release();
    return 0;
}
