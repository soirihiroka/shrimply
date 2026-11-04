#pragma once

// Keep NVIDIA optional at process startup. Preserve the SDK's versioned ABI
// names before replacing calls, and cache each resolved symbol atomically.
#if defined(__linux__)
#include <cuda.h>
#include <cudaGL.h>
#include <dlfcn.h>
#include <pthread.h>

static pthread_once_t shrimply_cuda_once = PTHREAD_ONCE_INIT;
static void *shrimply_cuda_library;
static void shrimply_cuda_load(void) {
  shrimply_cuda_library = dlopen("libcuda.so.1", RTLD_NOW | RTLD_LOCAL);
}
static void *shrimply_cuda_symbol(const char *name) {
  pthread_once(&shrimply_cuda_once, shrimply_cuda_load);
  return shrimply_cuda_library ? dlsym(shrimply_cuda_library, name) : NULL;
}
#define SHRIMPLY_CUDA_STRING_INNER(value) #value
#define SHRIMPLY_CUDA_STRING(value) SHRIMPLY_CUDA_STRING_INNER(value)
#define SHRIMPLY_CUDA_CALL(name, ...)                                          \
  ({                                                                           \
    static void *shrimply_cached;                                              \
    void *shrimply_symbol =                                                    \
        __atomic_load_n(&shrimply_cached, __ATOMIC_ACQUIRE);                   \
    if (!shrimply_symbol) {                                                    \
      shrimply_symbol = shrimply_cuda_symbol(shrimply_cuda_name_##name);       \
      if (shrimply_symbol)                                                     \
        __atomic_store_n(&shrimply_cached, shrimply_symbol, __ATOMIC_RELEASE); \
    }                                                                          \
    shrimply_cuda_type_##name shrimply_dynamic_function =                      \
        (shrimply_cuda_type_##name)shrimply_symbol;                            \
    shrimply_dynamic_function ? shrimply_dynamic_function(__VA_ARGS__)         \
                              : CUDA_ERROR_NOT_INITIALIZED;                    \
  })

typedef __typeof__(&cuCtxGetCurrent) shrimply_cuda_type_cuCtxGetCurrent;
static const char shrimply_cuda_name_cuCtxGetCurrent[] =
    SHRIMPLY_CUDA_STRING(cuCtxGetCurrent);
#undef cuCtxGetCurrent
#define cuCtxGetCurrent(...) SHRIMPLY_CUDA_CALL(cuCtxGetCurrent, __VA_ARGS__)

typedef __typeof__(&cuCtxPopCurrent) shrimply_cuda_type_cuCtxPopCurrent;
static const char shrimply_cuda_name_cuCtxPopCurrent[] =
    SHRIMPLY_CUDA_STRING(cuCtxPopCurrent);
#undef cuCtxPopCurrent
#define cuCtxPopCurrent(...) SHRIMPLY_CUDA_CALL(cuCtxPopCurrent, __VA_ARGS__)

typedef __typeof__(&cuCtxPushCurrent) shrimply_cuda_type_cuCtxPushCurrent;
static const char shrimply_cuda_name_cuCtxPushCurrent[] =
    SHRIMPLY_CUDA_STRING(cuCtxPushCurrent);
#undef cuCtxPushCurrent
#define cuCtxPushCurrent(...) SHRIMPLY_CUDA_CALL(cuCtxPushCurrent, __VA_ARGS__)

typedef __typeof__(&cuCtxSetCurrent) shrimply_cuda_type_cuCtxSetCurrent;
static const char shrimply_cuda_name_cuCtxSetCurrent[] =
    SHRIMPLY_CUDA_STRING(cuCtxSetCurrent);
#undef cuCtxSetCurrent
#define cuCtxSetCurrent(...) SHRIMPLY_CUDA_CALL(cuCtxSetCurrent, __VA_ARGS__)

typedef __typeof__(&cuCtxSynchronize) shrimply_cuda_type_cuCtxSynchronize;
static const char shrimply_cuda_name_cuCtxSynchronize[] =
    SHRIMPLY_CUDA_STRING(cuCtxSynchronize);
#undef cuCtxSynchronize
#define cuCtxSynchronize(...) SHRIMPLY_CUDA_CALL(cuCtxSynchronize, __VA_ARGS__)

typedef __typeof__(&cuDestroyExternalMemory)
    shrimply_cuda_type_cuDestroyExternalMemory;
static const char shrimply_cuda_name_cuDestroyExternalMemory[] =
    SHRIMPLY_CUDA_STRING(cuDestroyExternalMemory);
#undef cuDestroyExternalMemory
#define cuDestroyExternalMemory(...)                                           \
  SHRIMPLY_CUDA_CALL(cuDestroyExternalMemory, __VA_ARGS__)

typedef __typeof__(&cuDestroyExternalSemaphore)
    shrimply_cuda_type_cuDestroyExternalSemaphore;
static const char shrimply_cuda_name_cuDestroyExternalSemaphore[] =
    SHRIMPLY_CUDA_STRING(cuDestroyExternalSemaphore);
#undef cuDestroyExternalSemaphore
#define cuDestroyExternalSemaphore(...)                                        \
  SHRIMPLY_CUDA_CALL(cuDestroyExternalSemaphore, __VA_ARGS__)

typedef __typeof__(&cuDeviceGet) shrimply_cuda_type_cuDeviceGet;
static const char shrimply_cuda_name_cuDeviceGet[] =
    SHRIMPLY_CUDA_STRING(cuDeviceGet);
#undef cuDeviceGet
#define cuDeviceGet(...) SHRIMPLY_CUDA_CALL(cuDeviceGet, __VA_ARGS__)

typedef __typeof__(&cuDeviceGetAttribute)
    shrimply_cuda_type_cuDeviceGetAttribute;
static const char shrimply_cuda_name_cuDeviceGetAttribute[] =
    SHRIMPLY_CUDA_STRING(cuDeviceGetAttribute);
#undef cuDeviceGetAttribute
#define cuDeviceGetAttribute(...)                                              \
  SHRIMPLY_CUDA_CALL(cuDeviceGetAttribute, __VA_ARGS__)

typedef __typeof__(&cuDeviceGetUuid) shrimply_cuda_type_cuDeviceGetUuid;
static const char shrimply_cuda_name_cuDeviceGetUuid[] =
    SHRIMPLY_CUDA_STRING(cuDeviceGetUuid);
#undef cuDeviceGetUuid
#define cuDeviceGetUuid(...) SHRIMPLY_CUDA_CALL(cuDeviceGetUuid, __VA_ARGS__)

typedef __typeof__(&cuDevicePrimaryCtxGetState)
    shrimply_cuda_type_cuDevicePrimaryCtxGetState;
static const char shrimply_cuda_name_cuDevicePrimaryCtxGetState[] =
    SHRIMPLY_CUDA_STRING(cuDevicePrimaryCtxGetState);
#undef cuDevicePrimaryCtxGetState
#define cuDevicePrimaryCtxGetState(...)                                        \
  SHRIMPLY_CUDA_CALL(cuDevicePrimaryCtxGetState, __VA_ARGS__)

typedef __typeof__(&cuDevicePrimaryCtxRelease)
    shrimply_cuda_type_cuDevicePrimaryCtxRelease;
static const char shrimply_cuda_name_cuDevicePrimaryCtxRelease[] =
    SHRIMPLY_CUDA_STRING(cuDevicePrimaryCtxRelease);
#undef cuDevicePrimaryCtxRelease
#define cuDevicePrimaryCtxRelease(...)                                         \
  SHRIMPLY_CUDA_CALL(cuDevicePrimaryCtxRelease, __VA_ARGS__)

typedef __typeof__(&cuDevicePrimaryCtxRetain)
    shrimply_cuda_type_cuDevicePrimaryCtxRetain;
static const char shrimply_cuda_name_cuDevicePrimaryCtxRetain[] =
    SHRIMPLY_CUDA_STRING(cuDevicePrimaryCtxRetain);
#undef cuDevicePrimaryCtxRetain
#define cuDevicePrimaryCtxRetain(...)                                          \
  SHRIMPLY_CUDA_CALL(cuDevicePrimaryCtxRetain, __VA_ARGS__)

typedef __typeof__(&cuDevicePrimaryCtxSetFlags)
    shrimply_cuda_type_cuDevicePrimaryCtxSetFlags;
static const char shrimply_cuda_name_cuDevicePrimaryCtxSetFlags[] =
    SHRIMPLY_CUDA_STRING(cuDevicePrimaryCtxSetFlags);
#undef cuDevicePrimaryCtxSetFlags
#define cuDevicePrimaryCtxSetFlags(...)                                        \
  SHRIMPLY_CUDA_CALL(cuDevicePrimaryCtxSetFlags, __VA_ARGS__)

typedef __typeof__(&cuEventCreate) shrimply_cuda_type_cuEventCreate;
static const char shrimply_cuda_name_cuEventCreate[] =
    SHRIMPLY_CUDA_STRING(cuEventCreate);
#undef cuEventCreate
#define cuEventCreate(...) SHRIMPLY_CUDA_CALL(cuEventCreate, __VA_ARGS__)

typedef __typeof__(&cuEventDestroy) shrimply_cuda_type_cuEventDestroy;
static const char shrimply_cuda_name_cuEventDestroy[] =
    SHRIMPLY_CUDA_STRING(cuEventDestroy);
#undef cuEventDestroy
#define cuEventDestroy(...) SHRIMPLY_CUDA_CALL(cuEventDestroy, __VA_ARGS__)

typedef __typeof__(&cuEventElapsedTime) shrimply_cuda_type_cuEventElapsedTime;
static const char shrimply_cuda_name_cuEventElapsedTime[] =
    SHRIMPLY_CUDA_STRING(cuEventElapsedTime);
#undef cuEventElapsedTime
#define cuEventElapsedTime(...)                                                \
  SHRIMPLY_CUDA_CALL(cuEventElapsedTime, __VA_ARGS__)

typedef __typeof__(&cuEventRecord) shrimply_cuda_type_cuEventRecord;
static const char shrimply_cuda_name_cuEventRecord[] =
    SHRIMPLY_CUDA_STRING(cuEventRecord);
#undef cuEventRecord
#define cuEventRecord(...) SHRIMPLY_CUDA_CALL(cuEventRecord, __VA_ARGS__)

typedef __typeof__(&cuEventSynchronize) shrimply_cuda_type_cuEventSynchronize;
static const char shrimply_cuda_name_cuEventSynchronize[] =
    SHRIMPLY_CUDA_STRING(cuEventSynchronize);
#undef cuEventSynchronize
#define cuEventSynchronize(...)                                                \
  SHRIMPLY_CUDA_CALL(cuEventSynchronize, __VA_ARGS__)

typedef __typeof__(&cuExternalMemoryGetMappedBuffer)
    shrimply_cuda_type_cuExternalMemoryGetMappedBuffer;
static const char shrimply_cuda_name_cuExternalMemoryGetMappedBuffer[] =
    SHRIMPLY_CUDA_STRING(cuExternalMemoryGetMappedBuffer);
#undef cuExternalMemoryGetMappedBuffer
#define cuExternalMemoryGetMappedBuffer(...)                                   \
  SHRIMPLY_CUDA_CALL(cuExternalMemoryGetMappedBuffer, __VA_ARGS__)

typedef __typeof__(&cuExternalMemoryGetMappedMipmappedArray)
    shrimply_cuda_type_cuExternalMemoryGetMappedMipmappedArray;
static const char shrimply_cuda_name_cuExternalMemoryGetMappedMipmappedArray[] =
    SHRIMPLY_CUDA_STRING(cuExternalMemoryGetMappedMipmappedArray);
#undef cuExternalMemoryGetMappedMipmappedArray
#define cuExternalMemoryGetMappedMipmappedArray(...)                           \
  SHRIMPLY_CUDA_CALL(cuExternalMemoryGetMappedMipmappedArray, __VA_ARGS__)

typedef __typeof__(&cuGetErrorName) shrimply_cuda_type_cuGetErrorName;
static const char shrimply_cuda_name_cuGetErrorName[] =
    SHRIMPLY_CUDA_STRING(cuGetErrorName);
#undef cuGetErrorName
#define cuGetErrorName(...) SHRIMPLY_CUDA_CALL(cuGetErrorName, __VA_ARGS__)

typedef __typeof__(&cuGetErrorString) shrimply_cuda_type_cuGetErrorString;
static const char shrimply_cuda_name_cuGetErrorString[] =
    SHRIMPLY_CUDA_STRING(cuGetErrorString);
#undef cuGetErrorString
#define cuGetErrorString(...) SHRIMPLY_CUDA_CALL(cuGetErrorString, __VA_ARGS__)

typedef __typeof__(&cuGraphicsGLRegisterImage)
    shrimply_cuda_type_cuGraphicsGLRegisterImage;
static const char shrimply_cuda_name_cuGraphicsGLRegisterImage[] =
    SHRIMPLY_CUDA_STRING(cuGraphicsGLRegisterImage);
#undef cuGraphicsGLRegisterImage
#define cuGraphicsGLRegisterImage(...)                                         \
  SHRIMPLY_CUDA_CALL(cuGraphicsGLRegisterImage, __VA_ARGS__)

typedef __typeof__(&cuGraphicsMapResources)
    shrimply_cuda_type_cuGraphicsMapResources;
static const char shrimply_cuda_name_cuGraphicsMapResources[] =
    SHRIMPLY_CUDA_STRING(cuGraphicsMapResources);
#undef cuGraphicsMapResources
#define cuGraphicsMapResources(...)                                            \
  SHRIMPLY_CUDA_CALL(cuGraphicsMapResources, __VA_ARGS__)

typedef __typeof__(&cuGraphicsSubResourceGetMappedArray)
    shrimply_cuda_type_cuGraphicsSubResourceGetMappedArray;
static const char shrimply_cuda_name_cuGraphicsSubResourceGetMappedArray[] =
    SHRIMPLY_CUDA_STRING(cuGraphicsSubResourceGetMappedArray);
#undef cuGraphicsSubResourceGetMappedArray
#define cuGraphicsSubResourceGetMappedArray(...)                               \
  SHRIMPLY_CUDA_CALL(cuGraphicsSubResourceGetMappedArray, __VA_ARGS__)

typedef __typeof__(&cuGraphicsUnmapResources)
    shrimply_cuda_type_cuGraphicsUnmapResources;
static const char shrimply_cuda_name_cuGraphicsUnmapResources[] =
    SHRIMPLY_CUDA_STRING(cuGraphicsUnmapResources);
#undef cuGraphicsUnmapResources
#define cuGraphicsUnmapResources(...)                                          \
  SHRIMPLY_CUDA_CALL(cuGraphicsUnmapResources, __VA_ARGS__)

typedef __typeof__(&cuGraphicsUnregisterResource)
    shrimply_cuda_type_cuGraphicsUnregisterResource;
static const char shrimply_cuda_name_cuGraphicsUnregisterResource[] =
    SHRIMPLY_CUDA_STRING(cuGraphicsUnregisterResource);
#undef cuGraphicsUnregisterResource
#define cuGraphicsUnregisterResource(...)                                      \
  SHRIMPLY_CUDA_CALL(cuGraphicsUnregisterResource, __VA_ARGS__)

typedef __typeof__(&cuImportExternalMemory)
    shrimply_cuda_type_cuImportExternalMemory;
static const char shrimply_cuda_name_cuImportExternalMemory[] =
    SHRIMPLY_CUDA_STRING(cuImportExternalMemory);
#undef cuImportExternalMemory
#define cuImportExternalMemory(...)                                            \
  SHRIMPLY_CUDA_CALL(cuImportExternalMemory, __VA_ARGS__)

typedef __typeof__(&cuImportExternalSemaphore)
    shrimply_cuda_type_cuImportExternalSemaphore;
static const char shrimply_cuda_name_cuImportExternalSemaphore[] =
    SHRIMPLY_CUDA_STRING(cuImportExternalSemaphore);
#undef cuImportExternalSemaphore
#define cuImportExternalSemaphore(...)                                         \
  SHRIMPLY_CUDA_CALL(cuImportExternalSemaphore, __VA_ARGS__)

typedef __typeof__(&cuInit) shrimply_cuda_type_cuInit;
static const char shrimply_cuda_name_cuInit[] = SHRIMPLY_CUDA_STRING(cuInit);
#undef cuInit
#define cuInit(...) SHRIMPLY_CUDA_CALL(cuInit, __VA_ARGS__)

typedef __typeof__(&cuLaunchKernel) shrimply_cuda_type_cuLaunchKernel;
static const char shrimply_cuda_name_cuLaunchKernel[] =
    SHRIMPLY_CUDA_STRING(cuLaunchKernel);
#undef cuLaunchKernel
#define cuLaunchKernel(...) SHRIMPLY_CUDA_CALL(cuLaunchKernel, __VA_ARGS__)

typedef __typeof__(&cuMemAdvise) shrimply_cuda_type_cuMemAdvise;
static const char shrimply_cuda_name_cuMemAdvise[] =
    SHRIMPLY_CUDA_STRING(cuMemAdvise);
#undef cuMemAdvise
#define cuMemAdvise(...) SHRIMPLY_CUDA_CALL(cuMemAdvise, __VA_ARGS__)

typedef __typeof__(&cuMemAlloc) shrimply_cuda_type_cuMemAlloc;
static const char shrimply_cuda_name_cuMemAlloc[] =
    SHRIMPLY_CUDA_STRING(cuMemAlloc);
#undef cuMemAlloc
#define cuMemAlloc(...) SHRIMPLY_CUDA_CALL(cuMemAlloc, __VA_ARGS__)

typedef __typeof__(&cuMemAllocManaged) shrimply_cuda_type_cuMemAllocManaged;
static const char shrimply_cuda_name_cuMemAllocManaged[] =
    SHRIMPLY_CUDA_STRING(cuMemAllocManaged);
#undef cuMemAllocManaged
#define cuMemAllocManaged(...)                                                 \
  SHRIMPLY_CUDA_CALL(cuMemAllocManaged, __VA_ARGS__)

typedef __typeof__(&cuMemFree) shrimply_cuda_type_cuMemFree;
static const char shrimply_cuda_name_cuMemFree[] =
    SHRIMPLY_CUDA_STRING(cuMemFree);
#undef cuMemFree
#define cuMemFree(...) SHRIMPLY_CUDA_CALL(cuMemFree, __VA_ARGS__)

typedef __typeof__(&cuMemGetInfo) shrimply_cuda_type_cuMemGetInfo;
static const char shrimply_cuda_name_cuMemGetInfo[] =
    SHRIMPLY_CUDA_STRING(cuMemGetInfo);
#undef cuMemGetInfo
#define cuMemGetInfo(...) SHRIMPLY_CUDA_CALL(cuMemGetInfo, __VA_ARGS__)

typedef __typeof__(&cuMemPrefetchAsync) shrimply_cuda_type_cuMemPrefetchAsync;
static const char shrimply_cuda_name_cuMemPrefetchAsync[] =
    SHRIMPLY_CUDA_STRING(cuMemPrefetchAsync);
#undef cuMemPrefetchAsync
#define cuMemPrefetchAsync(...)                                                \
  SHRIMPLY_CUDA_CALL(cuMemPrefetchAsync, __VA_ARGS__)

typedef __typeof__(&cuMemcpy2D) shrimply_cuda_type_cuMemcpy2D;
static const char shrimply_cuda_name_cuMemcpy2D[] =
    SHRIMPLY_CUDA_STRING(cuMemcpy2D);
#undef cuMemcpy2D
#define cuMemcpy2D(...) SHRIMPLY_CUDA_CALL(cuMemcpy2D, __VA_ARGS__)

typedef __typeof__(&cuMemcpy2DAsync) shrimply_cuda_type_cuMemcpy2DAsync;
static const char shrimply_cuda_name_cuMemcpy2DAsync[] =
    SHRIMPLY_CUDA_STRING(cuMemcpy2DAsync);
#undef cuMemcpy2DAsync
#define cuMemcpy2DAsync(...) SHRIMPLY_CUDA_CALL(cuMemcpy2DAsync, __VA_ARGS__)

typedef __typeof__(&cuMemcpyDtoDAsync) shrimply_cuda_type_cuMemcpyDtoDAsync;
static const char shrimply_cuda_name_cuMemcpyDtoDAsync[] =
    SHRIMPLY_CUDA_STRING(cuMemcpyDtoDAsync);
#undef cuMemcpyDtoDAsync
#define cuMemcpyDtoDAsync(...)                                                 \
  SHRIMPLY_CUDA_CALL(cuMemcpyDtoDAsync, __VA_ARGS__)

typedef __typeof__(&cuMemcpyDtoHAsync) shrimply_cuda_type_cuMemcpyDtoHAsync;
static const char shrimply_cuda_name_cuMemcpyDtoHAsync[] =
    SHRIMPLY_CUDA_STRING(cuMemcpyDtoHAsync);
#undef cuMemcpyDtoHAsync
#define cuMemcpyDtoHAsync(...)                                                 \
  SHRIMPLY_CUDA_CALL(cuMemcpyDtoHAsync, __VA_ARGS__)

typedef __typeof__(&cuMemcpyHtoD) shrimply_cuda_type_cuMemcpyHtoD;
static const char shrimply_cuda_name_cuMemcpyHtoD[] =
    SHRIMPLY_CUDA_STRING(cuMemcpyHtoD);
#undef cuMemcpyHtoD
#define cuMemcpyHtoD(...) SHRIMPLY_CUDA_CALL(cuMemcpyHtoD, __VA_ARGS__)

typedef __typeof__(&cuMemcpyHtoDAsync) shrimply_cuda_type_cuMemcpyHtoDAsync;
static const char shrimply_cuda_name_cuMemcpyHtoDAsync[] =
    SHRIMPLY_CUDA_STRING(cuMemcpyHtoDAsync);
#undef cuMemcpyHtoDAsync
#define cuMemcpyHtoDAsync(...)                                                 \
  SHRIMPLY_CUDA_CALL(cuMemcpyHtoDAsync, __VA_ARGS__)

typedef __typeof__(&cuMemsetD8Async) shrimply_cuda_type_cuMemsetD8Async;
static const char shrimply_cuda_name_cuMemsetD8Async[] =
    SHRIMPLY_CUDA_STRING(cuMemsetD8Async);
#undef cuMemsetD8Async
#define cuMemsetD8Async(...) SHRIMPLY_CUDA_CALL(cuMemsetD8Async, __VA_ARGS__)

typedef __typeof__(&cuMipmappedArrayDestroy)
    shrimply_cuda_type_cuMipmappedArrayDestroy;
static const char shrimply_cuda_name_cuMipmappedArrayDestroy[] =
    SHRIMPLY_CUDA_STRING(cuMipmappedArrayDestroy);
#undef cuMipmappedArrayDestroy
#define cuMipmappedArrayDestroy(...)                                           \
  SHRIMPLY_CUDA_CALL(cuMipmappedArrayDestroy, __VA_ARGS__)

typedef __typeof__(&cuMipmappedArrayGetLevel)
    shrimply_cuda_type_cuMipmappedArrayGetLevel;
static const char shrimply_cuda_name_cuMipmappedArrayGetLevel[] =
    SHRIMPLY_CUDA_STRING(cuMipmappedArrayGetLevel);
#undef cuMipmappedArrayGetLevel
#define cuMipmappedArrayGetLevel(...)                                          \
  SHRIMPLY_CUDA_CALL(cuMipmappedArrayGetLevel, __VA_ARGS__)

typedef __typeof__(&cuModuleGetFunction) shrimply_cuda_type_cuModuleGetFunction;
static const char shrimply_cuda_name_cuModuleGetFunction[] =
    SHRIMPLY_CUDA_STRING(cuModuleGetFunction);
#undef cuModuleGetFunction
#define cuModuleGetFunction(...)                                               \
  SHRIMPLY_CUDA_CALL(cuModuleGetFunction, __VA_ARGS__)

typedef __typeof__(&cuModuleLoadData) shrimply_cuda_type_cuModuleLoadData;
static const char shrimply_cuda_name_cuModuleLoadData[] =
    SHRIMPLY_CUDA_STRING(cuModuleLoadData);
#undef cuModuleLoadData
#define cuModuleLoadData(...) SHRIMPLY_CUDA_CALL(cuModuleLoadData, __VA_ARGS__)

typedef __typeof__(&cuModuleUnload) shrimply_cuda_type_cuModuleUnload;
static const char shrimply_cuda_name_cuModuleUnload[] =
    SHRIMPLY_CUDA_STRING(cuModuleUnload);
#undef cuModuleUnload
#define cuModuleUnload(...) SHRIMPLY_CUDA_CALL(cuModuleUnload, __VA_ARGS__)

typedef __typeof__(&cuPointerGetAttribute)
    shrimply_cuda_type_cuPointerGetAttribute;
static const char shrimply_cuda_name_cuPointerGetAttribute[] =
    SHRIMPLY_CUDA_STRING(cuPointerGetAttribute);
#undef cuPointerGetAttribute
#define cuPointerGetAttribute(...)                                             \
  SHRIMPLY_CUDA_CALL(cuPointerGetAttribute, __VA_ARGS__)

typedef __typeof__(&cuStreamCreate) shrimply_cuda_type_cuStreamCreate;
static const char shrimply_cuda_name_cuStreamCreate[] =
    SHRIMPLY_CUDA_STRING(cuStreamCreate);
#undef cuStreamCreate
#define cuStreamCreate(...) SHRIMPLY_CUDA_CALL(cuStreamCreate, __VA_ARGS__)

typedef __typeof__(&cuStreamDestroy) shrimply_cuda_type_cuStreamDestroy;
static const char shrimply_cuda_name_cuStreamDestroy[] =
    SHRIMPLY_CUDA_STRING(cuStreamDestroy);
#undef cuStreamDestroy
#define cuStreamDestroy(...) SHRIMPLY_CUDA_CALL(cuStreamDestroy, __VA_ARGS__)

typedef __typeof__(&cuStreamSynchronize) shrimply_cuda_type_cuStreamSynchronize;
static const char shrimply_cuda_name_cuStreamSynchronize[] =
    SHRIMPLY_CUDA_STRING(cuStreamSynchronize);
#undef cuStreamSynchronize
#define cuStreamSynchronize(...)                                               \
  SHRIMPLY_CUDA_CALL(cuStreamSynchronize, __VA_ARGS__)

typedef __typeof__(&cuStreamWaitEvent) shrimply_cuda_type_cuStreamWaitEvent;
static const char shrimply_cuda_name_cuStreamWaitEvent[] =
    SHRIMPLY_CUDA_STRING(cuStreamWaitEvent);
#undef cuStreamWaitEvent
#define cuStreamWaitEvent(...)                                                 \
  SHRIMPLY_CUDA_CALL(cuStreamWaitEvent, __VA_ARGS__)

typedef __typeof__(&cuWaitExternalSemaphoresAsync)
    shrimply_cuda_type_cuWaitExternalSemaphoresAsync;
static const char shrimply_cuda_name_cuWaitExternalSemaphoresAsync[] =
    SHRIMPLY_CUDA_STRING(cuWaitExternalSemaphoresAsync);
#undef cuWaitExternalSemaphoresAsync
#define cuWaitExternalSemaphoresAsync(...)                                     \
  SHRIMPLY_CUDA_CALL(cuWaitExternalSemaphoresAsync, __VA_ARGS__)

#endif
