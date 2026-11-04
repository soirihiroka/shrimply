use std::mem;

use shrimply_gpu_cuda::{CudaContext, CudaStream, memory};
use shrimply_gpu_cuda_memory::GpuBuffer as DeviceBuffer;

const MAX_CACHED_PARAMETER_BUFFERS: usize = 4;

pub(super) fn copy<T>(
    context: &CudaContext,
    stream: &CudaStream,
    data: &[T],
    cache: &mut Vec<DeviceBuffer<T>>,
) -> Result<DeviceBuffer<T>, String> {
    let byte_len = mem::size_of_val(data);
    context
        .bind_to_thread()
        .map_err(|error| format!("bind CUDA context for params upload: {error:?}"))?;
    let buffer = if let Some(index) = cache.iter().position(|buffer| buffer.len() == data.len()) {
        cache.swap_remove(index)
    } else {
        // Keep common layer/motion counts resident when clips start or end,
        // without exposing stale trailing elements to the kernel's slice ABI.
        if cache.len() == MAX_CACHED_PARAMETER_BUFFERS {
            cache.remove(0);
        }
        shrimply_gpu_cuda_memory::global()
            .allocate_buffer::<u8>(
                stream,
                byte_len,
                shrimply_gpu_cuda_memory::AllocationClass::Transient,
                "CUDA compositor parameters",
            )?
            .cast_chunks::<T>()
            .map_err(|_| "CUDA compositor parameter buffer alignment mismatch".to_string())?
    };
    // New buffers are zeroed asynchronously on this stream. Order that memset
    // (and any previous use of a recycled buffer) before the legacy synchronous
    // host upload, which is not ordered with a non-blocking CUDA stream.
    if let Err(error) = stream.synchronize() {
        drop(buffer);
        return Err(format!("synchronize CUDA params buffer: {error:?}"));
    }
    if let Err(error) =
        unsafe { memory::memcpy_htod_sync(buffer.cu_deviceptr(), data.as_ptr(), byte_len) }
    {
        drop(buffer);
        return Err(format!("copy CUDA params: {error:?}"));
    }
    Ok(buffer)
}
