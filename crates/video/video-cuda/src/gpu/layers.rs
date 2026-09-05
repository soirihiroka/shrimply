use shrimply_cuda::{CudaStream, sys};

use crate::decode::DecodeControl;
use crate::layer::VideoLayer;
use shrimply_project::project::{CanvasSize, VideoSampleMethod};

use super::kernels::{LayerKind, Nv12LayerParams};

pub(super) struct PreparedLayers {
    pub(super) params: Vec<Nv12LayerParams>,
    pub(super) motion_transforms: Vec<glam::Mat3>,
    pub(super) frame_streams: Vec<sys::CUstream>,
    pub(super) anime4k: Vec<Anime4kRequest>,
}

pub(super) struct Anime4kRequest {
    pub(super) param_index: usize,
    pub(super) method: shrimply_anime4k::Method,
    pub(super) scale: f32,
}

pub(super) fn prepare(
    context: sys::CUcontext,
    stream: &CudaStream,
    canvas_size: CanvasSize,
    layers: &[VideoLayer],
    render_control: Option<&DecodeControl>,
) -> Result<PreparedLayers, String> {
    let mut params = Vec::with_capacity(layers.len());
    let mut motion_transforms = Vec::new();
    let mut frame_streams = Vec::with_capacity(layers.len());
    let mut anime4k = Vec::new();
    let canvas_width = canvas_size.width.max(1);

    for layer in layers {
        if render_control.is_some_and(DecodeControl::superseded) {
            return Err(super::RENDER_SUPERSEDED.to_string());
        }
        let (transform, motion_blur, sample_method, compositing, crop, padding, address_mode) =
            match layer {
                VideoLayer::Nv12 {
                    transform,
                    motion_blur,
                    sample_method,
                    compositing,
                    crop,
                    padding,
                    address_mode,
                    ..
                }
                | VideoLayer::Rgba {
                    transform,
                    motion_blur,
                    sample_method,
                    compositing,
                    crop,
                    padding,
                    address_mode,
                    ..
                } => (
                    transform,
                    motion_blur,
                    *sample_method,
                    *compositing,
                    *crop,
                    *padding,
                    *address_mode,
                ),
            };
        let opacity = compositing.opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            continue;
        }
        match layer {
            VideoLayer::Nv12 { frame, .. } => frame.prefetch_to_device(stream)?,
            VideoLayer::Rgba { layer, .. } => layer.prefetch_to_device(stream)?,
        }
        if render_control.is_some_and(DecodeControl::superseded) {
            return Err(super::RENDER_SUPERSEDED.to_string());
        }

        let motion_transform_offset = motion_transforms.len() as u32;
        let motion_sample_count = motion_blur
            .as_ref()
            .map_or(0, |samples| samples.len() as u32);
        let inverse = inverse_affine(*transform);
        if let Some(samples) = motion_blur {
            motion_transforms.extend(samples.iter().filter_map(|sample| inverse_affine(*sample)));
        }
        let motion_transform_count = motion_transforms.len() as u32 - motion_transform_offset;
        let inverse = match (inverse, motion_blur) {
            (Some(inverse), _) => inverse,
            (None, Some(_)) if motion_transform_count > 0 => {
                motion_transforms[motion_transform_offset as usize]
            }
            (None, _) => continue,
        };
        if motion_sample_count > 0 && motion_transform_count == 0 {
            continue;
        }

        let param_index = params.len();
        match layer {
            VideoLayer::Nv12 { frame, .. } => {
                if frame.format() != shrimply_visual_frame::VisualFormat::Nv12 {
                    return Err(format!(
                        "visual frame format {:?} is not supported by the CUDA compositor",
                        frame.format()
                    ));
                }

                let frame_context = frame
                    .context()
                    .ok_or_else(|| "CUDA video frame is missing its CUDA context".to_string())?;
                if frame_context.cu_ctx() != context {
                    return Err(
                        "CUDA video frame context does not match the active CUDA primary context"
                            .to_string(),
                    );
                }

                let y_plane = frame
                    .plane(0)
                    .ok_or_else(|| "CUDA video frame is missing the Y plane".to_string())?;
                let uv_plane = frame
                    .plane(1)
                    .ok_or_else(|| "CUDA video frame is missing the UV plane".to_string())?;
                let sample_method = if crate::math::is_pixel_aligned_translation(transform.matrix)
                    && crop.iter().all(|value| *value == 0.0)
                    && padding.iter().all(|value| *value == 0.0)
                    && matches!(sample_method, VideoSampleMethod::Nearest)
                {
                    VideoSampleMethod::Nearest
                } else {
                    sample_method
                };

                frame_streams.push(std::ptr::null_mut());
                params.push(Nv12LayerParams {
                    kind: LayerKind::Nv12,
                    y_plane: y_plane.device_ptr as usize as *const u8,
                    uv_plane: uv_plane.device_ptr as usize as *const u8,
                    rgba: std::ptr::null(),
                    y_pitch: y_plane.pitch_bytes,
                    uv_pitch: uv_plane.pitch_bytes,
                    rgba_pitch: 0,
                    source_width: frame.width(),
                    source_height: frame.height(),
                    canvas_width,
                    inverse,
                    motion_transform_offset,
                    motion_transform_count,
                    motion_sample_count,
                    opacity,
                    sample_method: super::kernels::sample_method(sample_method),
                    blend_mode: compositing.blend_mode,
                    crop,
                    padding,
                    address_mode: super::kernels::address_mode(address_mode),
                    _padding_0: [0; 4],
                });
            }
            VideoLayer::Rgba { layer, .. } => {
                let plane = layer.plane(0).expect("RGBA layer has no plane");
                params.push(Nv12LayerParams {
                    kind: LayerKind::Rgba,
                    y_plane: std::ptr::null(),
                    uv_plane: std::ptr::null(),
                    rgba: plane.device_ptr as usize as *const u32,
                    y_pitch: 0,
                    uv_pitch: 0,
                    rgba_pitch: plane.pitch_bytes,
                    source_width: layer.width(),
                    source_height: layer.height(),
                    canvas_width,
                    inverse,
                    motion_transform_offset,
                    motion_transform_count,
                    motion_sample_count,
                    opacity,
                    sample_method: super::kernels::sample_method(sample_method),
                    blend_mode: compositing.blend_mode,
                    crop,
                    padding,
                    address_mode: super::kernels::address_mode(address_mode),
                    _padding_0: [0; 4],
                });
            }
        }
        let method = match sample_method {
            VideoSampleMethod::Anime4k => Some(shrimply_anime4k::Method::CnnM),
            VideoSampleMethod::Anime4kSrgan => Some(shrimply_anime4k::Method::SrganUul),
            _ => None,
        };
        if let Some(method) = method {
            anime4k.push(Anime4kRequest {
                param_index,
                method,
                scale: motion_blur
                    .iter()
                    .flat_map(|samples| samples.iter())
                    .map(|sample| shrimply_math_geometry::max_affine_scale(sample.matrix))
                    .fold(
                        shrimply_math_geometry::max_affine_scale(transform.matrix),
                        f32::max,
                    ),
            });
        }
    }

    Ok(PreparedLayers {
        params,
        motion_transforms,
        frame_streams,
        anime4k,
    })
}

pub(super) fn apply_anime4k(
    prepared: &mut PreparedLayers,
    workspace: &mut shrimply_anime4k::Workspace,
    stream: &std::sync::Arc<shrimply_cuda::CudaStream>,
) -> Result<Vec<shrimply_gpu_memory::GpuBuffer<u32>>, String> {
    let mut buffers = Vec::with_capacity(prepared.anime4k.len());
    for request in &prepared.anime4k {
        let params = &mut prepared.params[request.param_index];
        let source = match params.kind {
            LayerKind::Nv12 => shrimply_anime4k::Source::Nv12 {
                y_plane: params.y_plane,
                uv_plane: params.uv_plane,
                y_pitch: params.y_pitch,
                uv_pitch: params.uv_pitch,
                width: params.source_width,
                height: params.source_height,
            },
            LayerKind::Rgba => shrimply_anime4k::Source::Rgba {
                pixels: params.rgba as *const u8,
                pitch: params.rgba_pitch,
                width: params.source_width,
                height: params.source_height,
            },
        };
        let Some(frame) = workspace.upscale(stream, source, request.method, request.scale)? else {
            params.sample_method = VideoSampleMethod::Bilinear;
            continue;
        };
        let scale_x = frame.width as f32 / params.source_width as f32;
        let scale_y = frame.height as f32 / params.source_height as f32;
        params.inverse = glam::Mat3::from_scale(glam::Vec2::new(scale_x, scale_y)) * params.inverse;
        let motion_start = params.motion_transform_offset as usize;
        let motion_end = motion_start + params.motion_transform_count as usize;
        for inverse in &mut prepared.motion_transforms[motion_start..motion_end] {
            *inverse = glam::Mat3::from_scale(glam::Vec2::new(scale_x, scale_y)) * *inverse;
        }
        params.padding[0] *= scale_y;
        params.padding[1] *= scale_x;
        params.padding[2] *= scale_y;
        params.padding[3] *= scale_x;
        params.kind = LayerKind::Rgba;
        params.y_plane = std::ptr::null();
        params.uv_plane = std::ptr::null();
        params.rgba = frame.pixels.cu_deviceptr() as usize as *const u32;
        params.y_pitch = 0;
        params.uv_pitch = 0;
        params.rgba_pitch = frame.width as usize * std::mem::size_of::<u32>();
        params.source_width = frame.width;
        params.source_height = frame.height;
        params.sample_method = VideoSampleMethod::Bilinear;
        buffers.push(frame.pixels);
    }
    Ok(buffers)
}

fn inverse_affine(transform: shrimply_math_geometry::ComposedTransform2D) -> Option<glam::Mat3> {
    shrimply_render_core::math::inverse_affine(transform.matrix)
}
