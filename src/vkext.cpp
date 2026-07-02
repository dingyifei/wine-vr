// vkext.cpp — enumerate what winevulkan/MoltenVK expose to a Windows-side Vulkan client:
//   device extensions (external memory/semaphore) + external-buffer handle-type capabilities.
// Loads vulkan-1.dll dynamically; no import lib needed.
#define VK_NO_PROTOTYPES
#define VK_USE_PLATFORM_WIN32_KHR
#include <windows.h>
#include <vulkan/vulkan.h>
#include <stdio.h>
#include <string.h>

int main(void){
    HMODULE vk = LoadLibraryA("vulkan-1.dll");
    if(!vk){ printf("FATAL: vulkan-1.dll not found\n"); return 1; }
    auto gipa = (PFN_vkGetInstanceProcAddr)GetProcAddress(vk,"vkGetInstanceProcAddr");
    auto vkCreateInstance = (PFN_vkCreateInstance)gipa(NULL,"vkCreateInstance");
    auto vkEnumerateInstanceExtensionProperties = (PFN_vkEnumerateInstanceExtensionProperties)gipa(NULL,"vkEnumerateInstanceExtensionProperties");

    // instance extensions
    uint32_t ic=0; vkEnumerateInstanceExtensionProperties(NULL,&ic,NULL);
    VkExtensionProperties* ie=new VkExtensionProperties[ic]; vkEnumerateInstanceExtensionProperties(NULL,&ic,ie);
    printf("=== instance extensions (%u) — external-capabilities ===\n", ic);
    for(uint32_t i=0;i<ic;i++) if(strstr(ie[i].extensionName,"external")) printf("  %s\n", ie[i].extensionName);

    const char* inst_exts[] = {"VK_KHR_get_physical_device_properties2",
                               "VK_KHR_external_memory_capabilities",
                               "VK_KHR_external_semaphore_capabilities"};
    VkApplicationInfo ai={VK_STRUCTURE_TYPE_APPLICATION_INFO}; ai.apiVersion=VK_API_VERSION_1_1;
    VkInstanceCreateInfo ci={VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO}; ci.pApplicationInfo=&ai;
    ci.enabledExtensionCount=3; ci.ppEnabledExtensionNames=inst_exts;
    VkInstance inst=NULL; VkResult r=vkCreateInstance(&ci,NULL,&inst);
    if(r!=VK_SUCCESS){ printf("vkCreateInstance=%d (retry no-ext)\n", r); ci.enabledExtensionCount=0; r=vkCreateInstance(&ci,NULL,&inst); }
    if(r!=VK_SUCCESS){ printf("FATAL vkCreateInstance=%d\n", r); return 2; }

    auto vkEnumeratePhysicalDevices=(PFN_vkEnumeratePhysicalDevices)gipa(inst,"vkEnumeratePhysicalDevices");
    auto vkGetPhysicalDeviceProperties=(PFN_vkGetPhysicalDeviceProperties)gipa(inst,"vkGetPhysicalDeviceProperties");
    auto vkEnumerateDeviceExtensionProperties=(PFN_vkEnumerateDeviceExtensionProperties)gipa(inst,"vkEnumerateDeviceExtensionProperties");
    auto vkGetPhysicalDeviceExternalBufferProperties=(PFN_vkGetPhysicalDeviceExternalBufferProperties)gipa(inst,"vkGetPhysicalDeviceExternalBufferProperties");
    auto vkGetPhysicalDeviceExternalSemaphoreProperties=(PFN_vkGetPhysicalDeviceExternalSemaphoreProperties)gipa(inst,"vkGetPhysicalDeviceExternalSemaphoreProperties");

    uint32_t pc=0; vkEnumeratePhysicalDevices(inst,&pc,NULL);
    VkPhysicalDevice* pd=new VkPhysicalDevice[pc]; vkEnumeratePhysicalDevices(inst,&pc,pd);
    printf("=== %u physical device(s) ===\n", pc);
    for(uint32_t d=0; d<pc; d++){
        VkPhysicalDeviceProperties p; vkGetPhysicalDeviceProperties(pd[d],&p);
        printf("device: %s (api %u.%u.%u)\n", p.deviceName,
               VK_VERSION_MAJOR(p.apiVersion),VK_VERSION_MINOR(p.apiVersion),VK_VERSION_PATCH(p.apiVersion));
        uint32_t ec=0; vkEnumerateDeviceExtensionProperties(pd[d],NULL,&ec,NULL);
        VkExtensionProperties* de=new VkExtensionProperties[ec]; vkEnumerateDeviceExtensionProperties(pd[d],NULL,&ec,de);
        printf("  external-related device extensions:\n");
        for(uint32_t i=0;i<ec;i++) if(strstr(de[i].extensionName,"external")||strstr(de[i].extensionName,"win32")||strstr(de[i].extensionName,"_fd"))
            printf("    %s\n", de[i].extensionName);

        // external BUFFER handle-type capability probe
        struct { const char* name; VkExternalMemoryHandleTypeFlagBits bit; } hts[] = {
            {"OPAQUE_FD",        VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT},
            {"OPAQUE_WIN32",     VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32_BIT},
            {"D3D11_TEXTURE",    VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_TEXTURE_BIT},
            {"D3D11_TEXTURE_KMT",VK_EXTERNAL_MEMORY_HANDLE_TYPE_D3D11_TEXTURE_KMT_BIT},
        };
        printf("  external BUFFER handle-type support (export|import|dedicated):\n");
        for(auto&h:hts){
            VkPhysicalDeviceExternalBufferInfo bi={VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_BUFFER_INFO};
            bi.usage=VK_BUFFER_USAGE_TRANSFER_SRC_BIT; bi.handleType=h.bit;
            VkExternalBufferProperties bp={VK_STRUCTURE_TYPE_EXTERNAL_BUFFER_PROPERTIES};
            if(vkGetPhysicalDeviceExternalBufferProperties) vkGetPhysicalDeviceExternalBufferProperties(pd[d],&bi,&bp);
            auto f=bp.externalMemoryProperties.externalMemoryFeatures;
            printf("    %-18s export=%d import=%d dedicatedOnly=%d  (featuresMask=0x%X)\n", h.name,
                   !!(f&VK_EXTERNAL_MEMORY_FEATURE_EXPORTABLE_BIT),
                   !!(f&VK_EXTERNAL_MEMORY_FEATURE_IMPORTABLE_BIT),
                   !!(f&VK_EXTERNAL_MEMORY_FEATURE_DEDICATED_ONLY_BIT), f);
        }
        // external SEMAPHORE handle-type probe
        struct { const char* name; VkExternalSemaphoreHandleTypeFlagBits bit; } sts[] = {
            {"OPAQUE_FD",   VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD_BIT},
            {"OPAQUE_WIN32",VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_WIN32_BIT},
        };
        printf("  external SEMAPHORE handle-type support (export|import):\n");
        for(auto&h:sts){
            VkPhysicalDeviceExternalSemaphoreInfo si={VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_EXTERNAL_SEMAPHORE_INFO};
            si.handleType=h.bit;
            VkExternalSemaphoreProperties sp={VK_STRUCTURE_TYPE_EXTERNAL_SEMAPHORE_PROPERTIES};
            if(vkGetPhysicalDeviceExternalSemaphoreProperties) vkGetPhysicalDeviceExternalSemaphoreProperties(pd[d],&si,&sp);
            printf("    %-18s export=%d import=%d  (featuresMask=0x%X)\n", h.name,
                   !!(sp.externalSemaphoreFeatures&VK_EXTERNAL_SEMAPHORE_FEATURE_EXPORTABLE_BIT),
                   !!(sp.externalSemaphoreFeatures&VK_EXTERNAL_SEMAPHORE_FEATURE_IMPORTABLE_BIT),
                   sp.externalSemaphoreFeatures);
        }
    }
    printf("=== done ===\n");
    return 0;
}
