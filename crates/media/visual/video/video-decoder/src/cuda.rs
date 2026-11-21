use shrimply_gpu_cuda::sys as cuda_sys;
use std::ptr;

pub(crate) fn cuda_memory_info() -> Result<(usize, usize), String> {
    unsafe {
        if cuda_sys::cuInit(0) != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            return Err("could not initialize CUDA while checking decoder memory".to_string());
        }
        let mut device = 0;
        if cuda_sys::cuDeviceGet(&mut device, shrimply_gpu_cuda::selected_device() as i32)
            != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            return Err(
                "could not select the CUDA device while checking decoder memory".to_string(),
            );
        }
        let mut context = ptr::null_mut();
        if cuda_sys::cuDevicePrimaryCtxRetain(&mut context, device)
            != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            return Err("could not retain CUDA primary context while checking memory".to_string());
        }
        if cuda_sys::cuCtxPushCurrent_v2(context) != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            let _ = cuda_sys::cuDevicePrimaryCtxRelease_v2(device);
            return Err(
                "could not activate CUDA primary context while checking memory".to_string(),
            );
        }
        let mut free = 0;
        let mut total = 0;
        let result = cuda_sys::cuMemGetInfo_v2(&mut free, &mut total);
        let mut popped = ptr::null_mut();
        let pop = cuda_sys::cuCtxPopCurrent_v2(&mut popped);
        let release = cuda_sys::cuDevicePrimaryCtxRelease_v2(device);
        if result != cuda_sys::cudaError_enum_CUDA_SUCCESS
            || pop != cuda_sys::cudaError_enum_CUDA_SUCCESS
            || release != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            return Err("could not query or restore CUDA memory context".to_string());
        }
        Ok((free, total))
    }
}
