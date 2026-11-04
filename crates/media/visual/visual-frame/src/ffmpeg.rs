use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

use ffmpeg_next::sys;
use shrimply_gpu_cuda::{CudaContext, sys as cuda_sys};

use crate::{VisualFormat, VisualFrame, VisualPlane};

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
        let (row_bytes, heights) = cuda_plane_layout(format, width, height)?;
        let planes = std::array::from_fn::<_, 2, _>(|index| VisualPlane {
            device_ptr: raw.data[index] as usize as cuda_sys::CUdeviceptr,
            pitch_bytes: raw.linesize[index].max(0) as usize,
            width_bytes: row_bytes[index],
            height: heights[index],
        });
        context
            .bind_to_thread()
            .map_err(|error| format!("activate CUDA context for decoded frame: {error}"))?;
        // FFmpeg may still be copying the decoded surface on its own stream.
        let ready = unsafe { cuda_sys::cuStreamSynchronize(cuda.stream) };
        if ready != cuda_sys::cudaError_enum_CUDA_SUCCESS {
            return Err(format!("finish decoded CUDA video frame: {ready:?}"));
        }
        // AVFrame refs keep FFmpeg's CUDA pool allocation alive without another
        // image allocation or copy. Video::clone copies pixels, so use av_frame_clone.
        let retained = unsafe { sys::av_frame_clone(decoded.as_ptr()) };
        if retained.is_null() {
            return Err("Could not retain decoded CUDA video frame".into());
        }
        let owner = DecodedFrame {
            _frame: unsafe { ffmpeg_next::frame::Video::wrap(retained) },
            context: context.clone(),
        };
        unsafe {
            Self::from_external_gpu(
                context,
                visual_format(format)?,
                width,
                height,
                &planes,
                Box::new(owner),
            )
        }
    }
}

struct DecodedFrame {
    _frame: ffmpeg_next::frame::Video,
    context: std::sync::Arc<CudaContext>,
}

impl Drop for DecodedFrame {
    fn drop(&mut self) {
        // Match owned CUDA buffer lifetime guarantees: a decoder pool must not
        // reuse this surface while a compositor/cache stream still reads it.
        self.context
            .synchronize()
            .expect("Cannot release a decoded CUDA frame while GPU readers are pending");
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
