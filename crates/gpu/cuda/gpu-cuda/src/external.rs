use crate::{CudaContext, CudaStream, sys};
#[cfg(target_os = "linux")]
use std::os::fd::{IntoRawFd, OwnedFd};
#[cfg(windows)]
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::{ptr, sync::Arc};

pub struct ImageDescriptor {
    #[cfg(target_os = "linux")]
    pub fd: OwnedFd,
    #[cfg(target_os = "linux")]
    pub semaphore_fd: OwnedFd,
    #[cfg(windows)]
    pub handle: OwnedHandle,
    #[cfg(windows)]
    pub semaphore_handle: OwnedHandle,
    pub allocation_size: u64,
    pub width: u32,
    pub height: u32,
}

/// Owns CUDA imports of a dedicated RGBA8 image and timeline semaphore.
pub struct ImportedImage {
    context: Arc<CudaContext>,
    mipmapped_array: sys::CUmipmappedArray,
    external_memory: usize,
    external_semaphore: usize,
    array: sys::CUarray,
    stream: Arc<CudaStream>,
}

fn bind_context(context: &CudaContext, operation: &str) -> Result<(), String> {
    context
        .bind_to_thread()
        .map_err(|e| format!("{operation}: {e:?}"))
}
fn cuda_check(result: sys::CUresult, operation: &str) -> Result<(), String> {
    if result == sys::cudaError_enum_CUDA_SUCCESS {
        Ok(())
    } else {
        Err(format!("{operation}: CUDA error {result}"))
    }
}
impl ImportedImage {
    pub fn new(
        context: Arc<CudaContext>,
        stream: Arc<CudaStream>,
        exported: ImageDescriptor,
    ) -> Result<Self, String> {
        bind_context(&context, "bind CUDA context for external import")?;
        #[cfg(target_os = "linux")]
        let fd = exported.fd.into_raw_fd();
        #[cfg(windows)]
        let handle = exported.handle.as_raw_handle();
        let mut external_memory = ptr::null_mut();
        let memory_desc = sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC {
            #[cfg(target_os = "linux")]
            type_: sys::CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD,
            #[cfg(windows)]
            type_: sys::CUexternalMemoryHandleType_enum_CU_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_WIN32,
            #[cfg(target_os = "linux")]
            handle: sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 { fd },
            #[cfg(windows)]
            handle: sys::CUDA_EXTERNAL_MEMORY_HANDLE_DESC_st__bindgen_ty_1 {
                words: [handle as usize, 0],
            },
            size: exported.allocation_size,
            flags: sys::CUDA_EXTERNAL_MEMORY_DEDICATED,
            reserved: [0; 16],
        };
        let import_result = cuda_check(
            unsafe { sys::cuImportExternalMemory(&mut external_memory, &memory_desc) },
            "cuImportExternalMemory for external",
        );
        #[cfg(target_os = "linux")]
        if let Err(error) = import_result {
            unsafe { libc::close(fd) };
            return Err(error);
        }
        #[cfg(windows)]
        import_result?;
        let mut mipmapped_array = ptr::null_mut();
        let mipmapped_desc = sys::CUDA_EXTERNAL_MEMORY_MIPMAPPED_ARRAY_DESC {
            offset: 0,
            arrayDesc: sys::CUDA_ARRAY3D_DESCRIPTOR {
                Width: exported.width as usize,
                Height: exported.height as usize,
                Depth: 0,
                Format: sys::CUarray_format_enum_CU_AD_FORMAT_UNSIGNED_INT8,
                NumChannels: 4,
                Flags: sys::CUDA_ARRAY3D_COLOR_ATTACHMENT | sys::CUDA_ARRAY3D_SURFACE_LDST,
            },
            numLevels: 1,
            reserved: [0; 16],
        };
        if let Err(error) = cuda_check(
            unsafe {
                sys::cuExternalMemoryGetMappedMipmappedArray(
                    &mut mipmapped_array,
                    external_memory,
                    &mipmapped_desc,
                )
            },
            "cuExternalMemoryGetMappedMipmappedArray for external",
        ) {
            if cuda_check(
                unsafe { sys::cuDestroyExternalMemory(external_memory) },
                "cuDestroyExternalMemory after external mapping failure",
            )
            .is_err()
            {
                std::process::abort();
            }
            return Err(error);
        }
        let mut array = ptr::null_mut();
        if let Err(error) = cuda_check(
            unsafe { sys::cuMipmappedArrayGetLevel(&mut array, mipmapped_array, 0) },
            "cuMipmappedArrayGetLevel for external",
        ) {
            if cuda_check(
                unsafe { sys::cuMipmappedArrayDestroy(mipmapped_array) },
                "cuMipmappedArrayDestroy after external level failure",
            )
            .and_then(|()| {
                cuda_check(
                    unsafe { sys::cuDestroyExternalMemory(external_memory) },
                    "cuDestroyExternalMemory after external level failure",
                )
            })
            .is_err()
            {
                std::process::abort();
            }
            return Err(error);
        }
        #[cfg(target_os = "linux")]
        let semaphore_fd = exported.semaphore_fd.into_raw_fd();
        #[cfg(windows)]
        let semaphore_handle = exported.semaphore_handle.as_raw_handle();
        let mut external_semaphore = ptr::null_mut();
        let semaphore_desc = sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC {
        #[cfg(target_os = "linux")]
        type_: sys::CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_TIMELINE_SEMAPHORE_FD,
        #[cfg(windows)]
        type_: sys::CUexternalSemaphoreHandleType_enum_CU_EXTERNAL_SEMAPHORE_HANDLE_TYPE_TIMELINE_SEMAPHORE_WIN32,
        #[cfg(target_os = "linux")]
        handle: sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1 {
            fd: semaphore_fd,
        },
        #[cfg(windows)]
        handle: sys::CUDA_EXTERNAL_SEMAPHORE_HANDLE_DESC_st__bindgen_ty_1 {
            words: [semaphore_handle as usize, 0],
        },
        flags: 0,
        reserved: [0; 16],
    };
        if let Err(error) = cuda_check(
            unsafe { sys::cuImportExternalSemaphore(&mut external_semaphore, &semaphore_desc) },
            "cuImportExternalSemaphore for external",
        ) {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::close(semaphore_fd)
            };
            if cuda_check(
                unsafe { sys::cuMipmappedArrayDestroy(mipmapped_array) },
                "cuMipmappedArrayDestroy after external semaphore import failure",
            )
            .and_then(|()| {
                cuda_check(
                    unsafe { sys::cuDestroyExternalMemory(external_memory) },
                    "cuDestroyExternalMemory after external semaphore import failure",
                )
            })
            .is_err()
            {
                std::process::abort();
            }
            return Err(error);
        }
        Ok(Self {
            context,
            mipmapped_array,
            external_memory: external_memory as usize,
            external_semaphore: external_semaphore as usize,
            array,
            stream,
        })
    }

    pub fn array(&self) -> sys::CUarray {
        self.array
    }

    pub fn synchronize(&self) -> Result<(), String> {
        self.stream
            .synchronize()
            .map_err(|error| format!("wait for imported image: {error:?}"))
    }

    pub fn wait(&self, stream: &CudaStream, semaphore_value: u64) -> Result<(), String> {
        bind_context(&self.context, "bind CUDA context for external image wait")?;
        let semaphores = [self.external_semaphore as sys::CUexternalSemaphore];
        let mut wait: sys::CUDA_EXTERNAL_SEMAPHORE_WAIT_PARAMS = unsafe { std::mem::zeroed() };
        wait.params.fence.value = semaphore_value;
        cuda_check(
            unsafe {
                sys::cuWaitExternalSemaphoresAsync(
                    semaphores.as_ptr(),
                    &wait,
                    1,
                    stream.cu_stream(),
                )
            },
            "wait for external image timeline semaphore",
        )
    }
}

impl Drop for ImportedImage {
    fn drop(&mut self) {
        if bind_context(&self.context, "bind CUDA context for external image drop").is_err()
            || self.stream.synchronize().is_err()
            || cuda_check(
                unsafe { sys::cuMipmappedArrayDestroy(self.mipmapped_array) },
                "cuMipmappedArrayDestroy for external image",
            )
            .is_err()
            || cuda_check(
                unsafe {
                    sys::cuDestroyExternalMemory(self.external_memory as sys::CUexternalMemory)
                },
                "cuDestroyExternalMemory for external image",
            )
            .is_err()
            || cuda_check(
                unsafe {
                    sys::cuDestroyExternalSemaphore(
                        self.external_semaphore as sys::CUexternalSemaphore,
                    )
                },
                "cuDestroyExternalSemaphore for external image",
            )
            .is_err()
        {
            std::process::abort();
        }
    }
}
