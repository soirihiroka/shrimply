use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use ffmpeg_next::sys;
use shrimply_gpu_cuda::{CudaContext, sys as cuda_sys};

use crate::{VisualFormat, VisualFrame};

#[derive(Clone, Copy)]
struct CudaContextInfo {
    context: cuda_sys::CUcontext,
    stream: cuda_sys::CUstream,
}

#[repr(C)]
struct AvCudaDeviceContext {
    cuda_ctx: cuda_sys::CUcontext,
    stream: cuda_sys::CUstream,
    internal: *mut c_void,
}

impl TryFrom<&ffmpeg_next::frame::Video> for VisualFrame {
    type Error = String;

    fn try_from(decoded: &ffmpeg_next::frame::Video) -> Result<Self, Self::Error> {
        let raw = unsafe { &*decoded.as_ptr() };
        if raw.format != sys::AVPixelFormat::AV_PIX_FMT_CUDA as i32 {
            return Err(format!(
                "NVIDIA decoder returned non-CUDA frame format {}; refusing CPU transfer",
                raw.format,
            ));
        }
        let frames_context = unsafe { cuda_frames_context(raw)? };
        let cuda = unsafe { cuda_context((*frames_context).device_ref) }?;
        let context = visual_frame_context()?;
        if context.cu_ctx() != cuda.context {
            return Err("FFmpeg and VisualFrame are not using the same CUDA context".to_string());
        }
        let width = raw.width.max(0) as u32;
        let height = raw.height.max(0) as u32;
        let format = unsafe { (*frames_context).sw_format };
        let frame = Self::allocate_persistent(
            context,
            visual_format(format)?,
            width,
            height,
            "retained decoded video frame",
        )?;
        let (row_bytes, heights) = cuda_plane_layout(format, width, height)?;
        let push = unsafe { cuda_sys::cuCtxPushCurrent_v2(cuda.context) };
        if push != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            return Err(format!("activate CUDA context for frame copy: {push:?}"));
        }
        let copy = (0..2).try_for_each(|index| {
            let mut descriptor: cuda_sys::CUDA_MEMCPY2D = unsafe { std::mem::zeroed() };
            descriptor.srcMemoryType = cuda_sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE;
            descriptor.srcDevice = raw.data[index] as usize as cuda_sys::CUdeviceptr;
            descriptor.srcPitch = raw.linesize[index].max(0) as usize;
            descriptor.dstMemoryType = match frame
                .memory_kind(index)
                .expect("video VisualFrame lost allocation metadata")
            {
                shrimply_gpu_cuda_memory::MemoryKind::Device => {
                    cuda_sys::CUmemorytype_enum_CU_MEMORYTYPE_DEVICE
                }
                shrimply_gpu_cuda_memory::MemoryKind::Managed => {
                    cuda_sys::CUmemorytype_enum_CU_MEMORYTYPE_UNIFIED
                }
            };
            let destination = frame
                .plane(index)
                .expect("video VisualFrame lost a required plane");
            descriptor.dstDevice = destination.device_ptr;
            descriptor.dstPitch = destination.pitch_bytes;
            descriptor.WidthInBytes = row_bytes[index];
            descriptor.Height = heights[index];
            let result = unsafe { cuda_sys::cuMemcpy2DAsync_v2(&descriptor, cuda.stream) };
            (result == cuda_sys::cudaError_enum_CUDA_SUCCESS)
                .then_some(())
                .ok_or_else(|| format!("copy decoded CUDA video plane: {result:?}"))
        });
        let synchronize = copy.and_then(|()| {
            let result = unsafe { cuda_sys::cuStreamSynchronize(cuda.stream) };
            (result == cuda_sys::cudaError_enum_CUDA_SUCCESS)
                .then_some(())
                .ok_or_else(|| format!("finish decoded CUDA video copy: {result:?}"))
        });
        let mut popped = ptr::null_mut();
        if unsafe { cuda_sys::cuCtxPopCurrent_v2(&mut popped) }
            != cuda_sys::cudaError_enum_CUDA_SUCCESS
        {
            std::process::abort();
        }
        synchronize?;
        Ok(frame)
    }
}

pub fn ffmpeg_cuda_context(frame: &ffmpeg_next::frame::Video) -> Option<cuda_sys::CUcontext> {
    let raw = unsafe { &*frame.as_ptr() };
    let frames = unsafe { cuda_frames_context(raw).ok()? };
    unsafe { cuda_context((*frames).device_ref).ok() }.map(|context| context.context)
}

unsafe fn cuda_frames_context(frame: &sys::AVFrame) -> Result<*mut sys::AVHWFramesContext, String> {
    let frames_context = if frame.hw_frames_ctx.is_null() {
        ptr::null_mut()
    } else {
        unsafe { (*frame.hw_frames_ctx).data.cast::<sys::AVHWFramesContext>() }
    };
    if frames_context.is_null() || unsafe { (*frames_context).device_ctx.is_null() } {
        return Err("CUDA video frame has no device context".to_string());
    }
    Ok(frames_context)
}

unsafe fn cuda_context(device_context: *mut sys::AVBufferRef) -> Result<CudaContextInfo, String> {
    let cuda = unsafe {
        (*device_context)
            .data
            .cast::<sys::AVHWDeviceContext>()
            .as_ref()
            .and_then(|device| device.hwctx.cast::<AvCudaDeviceContext>().as_ref())
    };
    let Some(cuda) = cuda.filter(|context| !context.cuda_ctx.is_null()) else {
        return Err("CUDA video frame has no CUDA context".to_string());
    };
    Ok(CudaContextInfo {
        context: cuda.cuda_ctx,
        stream: cuda.stream,
    })
}

fn cuda_plane_layout(
    format: sys::AVPixelFormat,
    width: u32,
    height: u32,
) -> Result<([usize; 2], [usize; 2]), String> {
    let bytes_per_sample = match format {
        sys::AVPixelFormat::AV_PIX_FMT_NV12 => 1,
        sys::AVPixelFormat::AV_PIX_FMT_P010LE => 2,
        _ => return Err(format!("unsupported retained CUDA frame format {format:?}")),
    };
    let row_bytes = (width as usize)
        .checked_mul(bytes_per_sample)
        .ok_or("retained frame row size overflow")?;
    Ok((
        [row_bytes, row_bytes],
        [height as usize, height.div_ceil(2) as usize],
    ))
}

fn visual_format(format: sys::AVPixelFormat) -> Result<VisualFormat, String> {
    match format {
        sys::AVPixelFormat::AV_PIX_FMT_NV12 => Ok(VisualFormat::Nv12),
        sys::AVPixelFormat::AV_PIX_FMT_P010LE => Ok(VisualFormat::P010),
        _ => Err(format!("unsupported retained CUDA frame format {format:?}")),
    }
}

fn visual_frame_context() -> Result<std::sync::Arc<CudaContext>, String> {
    static CONTEXT: OnceLock<Result<std::sync::Arc<CudaContext>, String>> = OnceLock::new();
    CONTEXT
        .get_or_init(|| {
            CudaContext::new(shrimply_gpu_cuda::selected_device())
                .map_err(|error| error.to_string())
        })
        .clone()
}
