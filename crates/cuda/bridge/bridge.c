#include <cuda.h>
#include <stddef.h>
#include <stdint.h>

CUresult shrimply_cuda_init(unsigned flags) { return cuInit(flags); }
CUresult shrimply_cuda_device_get(CUdevice *device, int ordinal) {
  return cuDeviceGet(device, ordinal);
}
CUresult shrimply_cuda_device_uuid(unsigned char *uuid, CUdevice device) {
  CUuuid value;
  CUresult result = cuDeviceGetUuid(&value, device);
  if (result == CUDA_SUCCESS) {
    for (size_t index = 0; index < sizeof value.bytes; ++index) {
      uuid[index] = (unsigned char)value.bytes[index];
    }
  }
  return result;
}
CUresult shrimply_cuda_primary_retain(void **context, CUdevice device) {
  return cuDevicePrimaryCtxRetain((CUcontext *)context, device);
}
CUresult shrimply_cuda_primary_release(CUdevice device) {
  return cuDevicePrimaryCtxRelease(device);
}
CUresult shrimply_cuda_primary_get_state(CUdevice device, unsigned *flags,
                                         int *active) {
  return cuDevicePrimaryCtxGetState(device, flags, active);
}
CUresult shrimply_cuda_primary_set_flags(CUdevice device, unsigned flags) {
  return cuDevicePrimaryCtxSetFlags(device, flags);
}
CUresult shrimply_cuda_context_get_current(void **context) {
  return cuCtxGetCurrent((CUcontext *)context);
}
CUresult shrimply_cuda_context_set_current(void *context) {
  return cuCtxSetCurrent((CUcontext)context);
}
CUresult shrimply_cuda_context_synchronize(void) { return cuCtxSynchronize(); }
CUresult shrimply_cuda_context_push(void *context) {
  return cuCtxPushCurrent((CUcontext)context);
}
CUresult shrimply_cuda_context_pop(void **context) {
  return cuCtxPopCurrent((CUcontext *)context);
}
CUresult shrimply_cuda_stream_create(void **stream) {
  return cuStreamCreate((CUstream *)stream, CU_STREAM_NON_BLOCKING);
}
CUresult shrimply_cuda_stream_destroy(void *stream) {
  return cuStreamDestroy((CUstream)stream);
}
CUresult shrimply_cuda_stream_synchronize(void *stream) {
  return cuStreamSynchronize((CUstream)stream);
}
CUresult shrimply_cuda_stream_wait_event(void *stream, void *event) {
  return cuStreamWaitEvent((CUstream)stream, (CUevent)event,
                           CU_EVENT_WAIT_DEFAULT);
}
CUresult shrimply_cuda_stream_wait_event_flags(void *stream, void *event,
                                               unsigned flags) {
  return cuStreamWaitEvent((CUstream)stream, (CUevent)event, flags);
}
CUresult shrimply_cuda_event_create(void **event, unsigned flags) {
  return cuEventCreate((CUevent *)event, flags);
}
CUresult shrimply_cuda_event_destroy(void *event) {
  return cuEventDestroy((CUevent)event);
}
CUresult shrimply_cuda_event_record(void *event, void *stream) {
  return cuEventRecord((CUevent)event, (CUstream)stream);
}
CUresult shrimply_cuda_event_synchronize(void *event) {
  return cuEventSynchronize((CUevent)event);
}
CUresult shrimply_cuda_event_elapsed(float *milliseconds, void *start,
                                     void *end) {
  return cuEventElapsedTime(milliseconds, (CUevent)start, (CUevent)end);
}
CUresult shrimply_cuda_module_load(void **module, const void *image) {
  return cuModuleLoadData((CUmodule *)module, image);
}
CUresult shrimply_cuda_module_unload(void *module) {
  return cuModuleUnload((CUmodule)module);
}
CUresult shrimply_cuda_module_function(void **function, void *module,
                                       const char *name) {
  return cuModuleGetFunction((CUfunction *)function, (CUmodule)module, name);
}
CUresult shrimply_cuda_launch(void *function, unsigned gx, unsigned gy,
                              unsigned gz, unsigned bx, unsigned by,
                              unsigned bz, unsigned shared, void *stream,
                              void **arguments) {
  return cuLaunchKernel((CUfunction)function, gx, gy, gz, bx, by, bz, shared,
                        (CUstream)stream, arguments, 0);
}
CUresult shrimply_cuda_mem_alloc(uint64_t *pointer, size_t bytes) {
  return cuMemAlloc((CUdeviceptr *)pointer, bytes);
}
CUresult shrimply_cuda_mem_free(uint64_t pointer) {
  return cuMemFree((CUdeviceptr)pointer);
}
CUresult shrimply_cuda_memcpy_htod_async(uint64_t destination,
                                         const void *source, size_t bytes,
                                         void *stream) {
  return cuMemcpyHtoDAsync((CUdeviceptr)destination, source, bytes,
                           (CUstream)stream);
}
CUresult shrimply_cuda_memcpy_htod(uint64_t destination, const void *source,
                                   size_t bytes) {
  return cuMemcpyHtoD((CUdeviceptr)destination, source, bytes);
}
CUresult shrimply_cuda_memcpy_dtoh_async(void *destination, uint64_t source,
                                         size_t bytes, void *stream) {
  return cuMemcpyDtoHAsync(destination, (CUdeviceptr)source, bytes,
                           (CUstream)stream);
}
CUresult shrimply_cuda_memcpy_dtod_async(uint64_t destination, uint64_t source,
                                         size_t bytes, void *stream) {
  return cuMemcpyDtoDAsync((CUdeviceptr)destination, (CUdeviceptr)source, bytes,
                           (CUstream)stream);
}
CUresult shrimply_cuda_memset_async(uint64_t destination, unsigned char value,
                                    size_t bytes, void *stream) {
  return cuMemsetD8Async((CUdeviceptr)destination, value, bytes,
                         (CUstream)stream);
}
CUresult shrimply_cuda_mem_alloc_managed(uint64_t *pointer, size_t bytes,
                                         unsigned flags) {
  return cuMemAllocManaged((CUdeviceptr *)pointer, bytes, flags);
}
CUresult shrimply_cuda_mem_get_info(size_t *free_bytes, size_t *total_bytes) {
  return cuMemGetInfo(free_bytes, total_bytes);
}
#if CUDA_VERSION >= 13020
CUresult shrimply_cuda_mem_advise_v2(uint64_t pointer, size_t bytes,
                                     unsigned advice, CUmemLocation location) {
  return cuMemAdvise((CUdeviceptr)pointer, bytes, (CUmem_advise)advice,
                     location);
}
CUresult shrimply_cuda_mem_prefetch_v2(uint64_t pointer, size_t bytes,
                                       CUmemLocation location, unsigned flags,
                                       void *stream) {
  return cuMemPrefetchAsync((CUdeviceptr)pointer, bytes, location, flags,
                            (CUstream)stream);
}
#else
CUresult shrimply_cuda_mem_advise(uint64_t pointer, size_t bytes,
                                  unsigned advice, CUdevice device) {
  return cuMemAdvise((CUdeviceptr)pointer, bytes, (CUmem_advise)advice, device);
}
CUresult shrimply_cuda_mem_prefetch(uint64_t pointer, size_t bytes,
                                    CUdevice device, void *stream) {
  return cuMemPrefetchAsync((CUdeviceptr)pointer, bytes, device,
                            (CUstream)stream);
}
#endif
CUresult shrimply_cuda_memcpy_2d(const void *descriptor) {
  return cuMemcpy2D((const CUDA_MEMCPY2D *)descriptor);
}
CUresult shrimply_cuda_memcpy_2d_async(const void *descriptor, void *stream) {
  return cuMemcpy2DAsync((const CUDA_MEMCPY2D *)descriptor, (CUstream)stream);
}
CUresult shrimply_cuda_pointer_get_attribute(void *data, unsigned attribute,
                                             uint64_t pointer) {
  return cuPointerGetAttribute(data, (CUpointer_attribute)attribute,
                               (CUdeviceptr)pointer);
}
CUresult shrimply_cuda_import_external_memory(void **memory,
                                              const void *descriptor) {
  return cuImportExternalMemory(
      (CUexternalMemory *)memory,
      (const CUDA_EXTERNAL_MEMORY_HANDLE_DESC *)descriptor);
}
CUresult shrimply_cuda_external_memory_get_buffer(uint64_t *pointer,
                                                  void *memory,
                                                  const void *descriptor) {
  return cuExternalMemoryGetMappedBuffer(
      (CUdeviceptr *)pointer, (CUexternalMemory)memory,
      (const CUDA_EXTERNAL_MEMORY_BUFFER_DESC *)descriptor);
}
CUresult
shrimply_cuda_external_memory_get_mipmapped_array(void **array, void *memory,
                                                  const void *descriptor) {
  return cuExternalMemoryGetMappedMipmappedArray(
      (CUmipmappedArray *)array, (CUexternalMemory)memory,
      (const CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC *)descriptor);
}
CUresult shrimply_cuda_destroy_external_memory(void *memory) {
  return cuDestroyExternalMemory((CUexternalMemory)memory);
}
CUresult shrimply_cuda_mipmapped_array_get_level(void **array, void *mipmapped,
                                                 unsigned level) {
  return cuMipmappedArrayGetLevel((CUarray *)array, (CUmipmappedArray)mipmapped,
                                  level);
}
CUresult shrimply_cuda_mipmapped_array_destroy(void *mipmapped) {
  return cuMipmappedArrayDestroy((CUmipmappedArray)mipmapped);
}
CUresult shrimply_cuda_import_external_semaphore(void **semaphore,
                                                 const void *descriptor) {
  return cuImportExternalSemaphore(
      (CUexternalSemaphore *)semaphore,
      (const CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC *)descriptor);
}
CUresult shrimply_cuda_wait_external_semaphores(const void *semaphores,
                                                const void *parameters,
                                                unsigned count, void *stream) {
  return cuWaitExternalSemaphoresAsync(
      (const CUexternalSemaphore *)semaphores,
      (const CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS *)parameters, count,
      (CUstream)stream);
}
CUresult shrimply_cuda_destroy_external_semaphore(void *semaphore) {
  return cuDestroyExternalSemaphore((CUexternalSemaphore)semaphore);
}
CUresult shrimply_cuda_graphics_map(unsigned count, void **resources,
                                    void *stream) {
  return cuGraphicsMapResources(count, (CUgraphicsResource *)resources,
                                (CUstream)stream);
}
CUresult shrimply_cuda_graphics_unmap(unsigned count, void **resources,
                                      void *stream) {
  return cuGraphicsUnmapResources(count, (CUgraphicsResource *)resources,
                                  (CUstream)stream);
}
CUresult shrimply_cuda_graphics_mapped_array(void **array, void *resource,
                                             unsigned array_index,
                                             unsigned mip_level) {
  return cuGraphicsSubResourceGetMappedArray(
      (CUarray *)array, (CUgraphicsResource)resource, array_index, mip_level);
}
CUresult shrimply_cuda_graphics_unregister(void *resource) {
  return cuGraphicsUnregisterResource((CUgraphicsResource)resource);
}
CUresult shrimply_cuda_error_name(CUresult result, const char **name) {
  return cuGetErrorName(result, name);
}
CUresult shrimply_cuda_error_string(CUresult result, const char **description) {
  return cuGetErrorString(result, description);
}
