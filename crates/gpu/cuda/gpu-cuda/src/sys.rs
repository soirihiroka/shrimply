#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]

use std::ffi::{c_char, c_void};

pub type CUresult = i32;
pub type CUdevice = i32;
pub type CUdeviceptr = u64;
pub type CUcontext = *mut c_void;
pub type CUstream = *mut c_void;
pub type CUevent = *mut c_void;
pub type CUmodule = *mut c_void;
pub type CUfunction = *mut c_void;
pub type CUarray = *mut c_void;
pub type CUmipmappedArray = *mut c_void;
pub type CUgraphicsResource = *mut c_void;
pub type CUexternalMemory = *mut c_void;
pub type CUexternalSemaphore = *mut c_void;
pub const CUDA_SUCCESS: CUresult = 0;
pub const cudaError_enum_CUDA_SUCCESS: CUresult = 0;
pub const cudaError_enum_CUDA_ERROR_OUT_OF_MEMORY: CUresult = 2;
pub const CU_EVENT_DEFAULT: u32 = 0;
pub const CU_EVENT_DISABLE_TIMING: u32 = 2;
pub const CUevent_flags_enum_CU_EVENT_DEFAULT: u32 = 0;
pub const CUevent_flags_enum_CU_EVENT_DISABLE_TIMING: u32 = 2;
pub type CUevent_flags = u32;
pub const CUmemAttach_flags_enum_CU_MEM_ATTACH_GLOBAL: u32 = 1;
pub const CUmem_advise_enum_CU_MEM_ADVISE_SET_PREFERRED_LOCATION: u32 = 3;
pub const CUmemLocationType_enum_CU_MEM_LOCATION_TYPE_HOST: u32 = 1;
pub const CUmemLocationType_enum_CU_MEM_LOCATION_TYPE_DEVICE: u32 = 2;
pub type CUmemorytype = u32;
pub const CUmemorytype_enum_CU_MEMORYTYPE_HOST: CUmemorytype = 1;
pub const CUmemorytype_enum_CU_MEMORYTYPE_DEVICE: CUmemorytype = 2;
pub const CUmemorytype_enum_CU_MEMORYTYPE_ARRAY: CUmemorytype = 3;
pub const CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED: CUmemorytype = 4;
pub const CUDA_EXTERNAL_MEMORY_DEDICATED: u32 = 1;
pub const CUDA_ARRAY3D_SURFACE_LDST: u32 = 2;
pub const CUDA_ARRAY3D_COLOR_ATTACHMENT: u32 = 32;
pub const CUarray_format_enum_CU_AD_FORMAT_UNSIGNED_INT8: u32 = 1;
pub const CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD: u32 = 1;
pub const CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32: u32 = 2;
pub const CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD: u32 = 1;
pub const CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_WIN32: u32 = 2;
pub const CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_TIMELINE_SEMAPHORE_FD:u32=9;
pub const CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_TIMELINE_SEMAPHORE_WIN32:u32=10;
pub const CUevent_wait_flags_enum_CU_EVENT_WAIT_DEFAULT: u32 = 0;
pub const CUctx_flags_enum_CU_CTX_SCHED_BLOCKING_SYNC: u32 = 4;
pub const CUctx_flags_enum_CU_CTX_SCHED_MASK: u32 = 7;
pub const CUpointer_attribute_enum_CU_POINTER_ATTRIBUTE_MEMORY_TYPE: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_ARRAY3D_DESCRIPTOR {
    pub Width: usize,
    pub Height: usize,
    pub Depth: usize,
    pub Format: u32,
    pub NumChannels: u32,
    pub Flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 {
    pub fd: i32,
    pub words: [usize; 2],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_MEMORY_HANDLE_DESC {
    pub type_: u32,
    pub handle: CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1,
    pub size: u64,
    pub flags: u32,
    pub reserved: [u32; 16],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_MEMORY_BUFFER_DESC {
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
    pub reserved: [u32; 16],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC {
    pub offset: u64,
    pub arrayDesc: CUDA_ARRAY3D_DESCRIPTOR,
    pub numLevels: u32,
    pub reserved: [u32; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1 {
    pub fd: i32,
    pub words: [usize; 2],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC {
    pub type_: u32,
    pub handle: CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1,
    pub flags: u32,
    pub reserved: [u32; 16],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1__bindgen_ty_1 {
    pub value: u64,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1__bindgen_ty_3 {
    pub key: u64,
    pub timeoutMs: u32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1 {
    pub fence: CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1__bindgen_ty_1,
    pub nvSciSync: u64,
    pub keyedMutex: CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1__bindgen_ty_3,
    pub reserved: [u32; 10],
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS {
    pub params: CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS_st__bindgen_ty_1,
    pub flags: u32,
    pub reserved: [u32; 16],
}

const _: () = {
    assert!(std::mem::size_of::<CUDA_ARRAY3D_DESCRIPTOR>() == 40);
    assert!(std::mem::size_of::<CUDA_EXTERNAL_MEMORY_HANDLE_DESC>() == 104);
    assert!(std::mem::size_of::<CUDA_EXTERNAL_MEMORY_BUFFER_DESC>() == 88);
    assert!(std::mem::size_of::<CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC>() == 120);
    assert!(std::mem::size_of::<CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC>() == 96);
    assert!(std::mem::size_of::<CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS>() == 144);
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUDA_MEMCPY2D {
    pub srcXInBytes: usize,
    pub srcY: usize,
    pub srcMemoryType: CUmemorytype,
    pub srcHost: *const c_void,
    pub srcDevice: CUdeviceptr,
    pub srcArray: CUarray,
    pub srcPitch: usize,
    pub dstXInBytes: usize,
    pub dstY: usize,
    pub dstMemoryType: CUmemorytype,
    pub dstHost: *mut c_void,
    pub dstDevice: CUdeviceptr,
    pub dstArray: CUarray,
    pub dstPitch: usize,
    pub WidthInBytes: usize,
    pub Height: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union CUmemLocation_st__bindgen_ty_1 {
    pub id: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CUmemLocation {
    pub type_: u32,
    pub __bindgen_anon_1: CUmemLocation_st__bindgen_ty_1,
}

unsafe extern "C" {
    #[link_name = "shrimply_cuda_init"]
    pub fn cuInit(flags: u32) -> CUresult;
    #[link_name = "shrimply_cuda_device_get"]
    pub fn cuDeviceGet(device: *mut CUdevice, ordinal: i32) -> CUresult;
    #[link_name = "shrimply_cuda_primary_retain"]
    pub fn cuDevicePrimaryCtxRetain(context: *mut CUcontext, device: CUdevice) -> CUresult;
    #[link_name = "shrimply_cuda_primary_release"]
    pub fn cuDevicePrimaryCtxRelease_v2(device: CUdevice) -> CUresult;
    #[link_name = "shrimply_cuda_primary_get_state"]
    pub fn cuDevicePrimaryCtxGetState(
        device: CUdevice,
        flags: *mut u32,
        active: *mut i32,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_primary_set_flags"]
    pub fn cuDevicePrimaryCtxSetFlags_v2(device: CUdevice, flags: u32) -> CUresult;
    #[link_name = "shrimply_cuda_error_name"]
    pub fn cuGetErrorName(result: CUresult, name: *mut *const c_char) -> CUresult;
    #[link_name = "shrimply_cuda_error_string"]
    pub fn cuGetErrorString(result: CUresult, description: *mut *const c_char) -> CUresult;
    #[link_name = "shrimply_cuda_mem_alloc_managed"]
    pub fn cuMemAllocManaged(pointer: *mut CUdeviceptr, bytes: usize, flags: u32) -> CUresult;
    #[link_name = "shrimply_cuda_mem_get_info"]
    pub fn cuMemGetInfo_v2(free_bytes: *mut usize, total_bytes: *mut usize) -> CUresult;
    #[link_name = "shrimply_cuda_mem_advise_v2"]
    pub fn cuMemAdvise_v2(
        pointer: CUdeviceptr,
        bytes: usize,
        advice: u32,
        location: CUmemLocation,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_mem_prefetch_v2"]
    pub fn cuMemPrefetchAsync_v2(
        pointer: CUdeviceptr,
        bytes: usize,
        location: CUmemLocation,
        flags: u32,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_mem_advise"]
    pub fn cuMemAdvise(
        pointer: CUdeviceptr,
        bytes: usize,
        advice: u32,
        device: CUdevice,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_mem_prefetch"]
    pub fn cuMemPrefetchAsync(
        pointer: CUdeviceptr,
        bytes: usize,
        device: CUdevice,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_context_push"]
    pub fn cuCtxPushCurrent_v2(context: CUcontext) -> CUresult;
    #[link_name = "shrimply_cuda_context_pop"]
    pub fn cuCtxPopCurrent_v2(context: *mut CUcontext) -> CUresult;
    #[link_name = "shrimply_cuda_memcpy_2d"]
    pub fn cuMemcpy2D_v2(descriptor: *const CUDA_MEMCPY2D) -> CUresult;
    #[link_name = "shrimply_cuda_memcpy_2d_async"]
    pub fn cuMemcpy2DAsync_v2(descriptor: *const CUDA_MEMCPY2D, stream: CUstream) -> CUresult;
    #[link_name = "shrimply_cuda_memcpy_dtod_async"]
    pub fn cuMemcpyDtoDAsync_v2(
        destination: CUdeviceptr,
        source: CUdeviceptr,
        bytes: usize,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_mem_free"]
    pub fn cuMemFree_v2(pointer: CUdeviceptr) -> CUresult;
    #[link_name = "shrimply_cuda_event_create"]
    pub fn cuEventCreate(event: *mut CUevent, flags: u32) -> CUresult;
    #[link_name = "shrimply_cuda_event_record"]
    pub fn cuEventRecord(event: CUevent, stream: CUstream) -> CUresult;
    #[link_name = "shrimply_cuda_event_destroy"]
    pub fn cuEventDestroy_v2(event: CUevent) -> CUresult;
    #[link_name = "shrimply_cuda_stream_wait_event_flags"]
    pub fn cuStreamWaitEvent(stream: CUstream, event: CUevent, flags: u32) -> CUresult;
    #[link_name = "shrimply_cuda_pointer_get_attribute"]
    pub fn cuPointerGetAttribute(
        data: *mut c_void,
        attribute: u32,
        pointer: CUdeviceptr,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_import_external_memory"]
    pub fn cuImportExternalMemory(
        memory: *mut CUexternalMemory,
        descriptor: *const CUDA_EXTERNAL_MEMORY_HANDLE_DESC,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_external_memory_get_buffer"]
    pub fn cuExternalMemoryGetMappedBuffer(
        pointer: *mut CUdeviceptr,
        memory: CUexternalMemory,
        descriptor: *const CUDA_EXTERNAL_MEMORY_BUFFER_DESC,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_external_memory_get_mipmapped_array"]
    pub fn cuExternalMemoryGetMappedMipmappedArray(
        array: *mut CUmipmappedArray,
        memory: CUexternalMemory,
        descriptor: *const CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_destroy_external_memory"]
    pub fn cuDestroyExternalMemory(memory: CUexternalMemory) -> CUresult;
    #[link_name = "shrimply_cuda_mipmapped_array_get_level"]
    pub fn cuMipmappedArrayGetLevel(
        array: *mut CUarray,
        mipmapped: CUmipmappedArray,
        level: u32,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_mipmapped_array_destroy"]
    pub fn cuMipmappedArrayDestroy(mipmapped: CUmipmappedArray) -> CUresult;
    #[link_name = "shrimply_cuda_import_external_semaphore"]
    pub fn cuImportExternalSemaphore(
        semaphore: *mut CUexternalSemaphore,
        descriptor: *const CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_wait_external_semaphores"]
    pub fn cuWaitExternalSemaphoresAsync(
        semaphores: *const CUexternalSemaphore,
        parameters: *const CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS,
        count: u32,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_destroy_external_semaphore"]
    pub fn cuDestroyExternalSemaphore(semaphore: CUexternalSemaphore) -> CUresult;
    #[link_name = "shrimply_cuda_graphics_map"]
    pub fn cuGraphicsMapResources(
        count: u32,
        resources: *mut CUgraphicsResource,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_graphics_unmap"]
    pub fn cuGraphicsUnmapResources(
        count: u32,
        resources: *mut CUgraphicsResource,
        stream: CUstream,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_graphics_mapped_array"]
    pub fn cuGraphicsSubResourceGetMappedArray(
        array: *mut CUarray,
        resource: CUgraphicsResource,
        array_index: u32,
        mip_level: u32,
    ) -> CUresult;
    #[link_name = "shrimply_cuda_graphics_unregister"]
    pub fn cuGraphicsUnregisterResource(resource: CUgraphicsResource) -> CUresult;
    #[link_name = "shrimply_cuda_stream_synchronize"]
    pub fn cuStreamSynchronize(stream: CUstream) -> CUresult;
}
