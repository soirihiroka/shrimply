use crate::{Buffer, Renderer, Submission, device::Device};
use ash::{vk, vk::Handle};
use ffmpeg_next::{self as ffmpeg, sys};
use std::{ptr, sync::Arc};

/// Keeps the decoder surface alive until the final GPU read completes.
pub struct VideoFrame {
    pub frame: ffmpeg::frame::Video,
}

pub fn decoder(
    parameters: &ffmpeg::codec::Parameters,
    time_base: ffmpeg::Rational,
) -> Result<ffmpeg::decoder::Video, String> {
    let device = Device::get()?;
    let codec = ffmpeg::codec::decoder::find(parameters.id()).ok_or("Video decoder not found")?;
    let mut supported = false;
    for index in 0.. {
        let config = unsafe { sys::avcodec_get_hw_config(codec.as_ptr(), index) };
        if config.is_null() {
            break;
        }
        if unsafe { (*config).device_type == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI } {
            supported = true;
            break;
        }
    }
    if !supported {
        return Err(format!(
            "Codec {:?} does not expose VA-API decoding",
            parameters.id()
        ));
    }
    let mut context = ffmpeg::codec::context::Context::from_parameters(parameters.clone())
        .map_err(|e| e.to_string())?
        .decoder();
    context.set_packet_time_base(time_base);
    let reference = unsafe { sys::av_buffer_ref(device.va) };
    if reference.is_null() {
        return Err("Could not retain Intel VA-API context".into());
    }
    unsafe {
        (*context.as_mut_ptr()).hw_device_ctx = reference;
        (*context.as_mut_ptr()).get_format = Some(format);
    }
    context
        .open_as(codec)
        .and_then(|context| context.video())
        .map_err(|e| format!("Open Intel video decoder: {e}"))
}

unsafe extern "C" fn format(
    _: *mut sys::AVCodecContext,
    formats: *const sys::AVPixelFormat,
) -> sys::AVPixelFormat {
    let mut format = formats;
    while !format.is_null() && unsafe { *format } != sys::AVPixelFormat::AV_PIX_FMT_NONE {
        if unsafe { *format } == sys::AVPixelFormat::AV_PIX_FMT_VAAPI {
            return unsafe { *format };
        }
        format = unsafe { format.add(1) };
    }
    sys::AVPixelFormat::AV_PIX_FMT_NONE
}

impl Renderer {
    /// Import the VA-API surface through DRM PRIME and copy its planes on the GPU.
    /// Pixels never enter CPU-addressable staging storage.
    pub fn video(&mut self, source: &Arc<VideoFrame>) -> Result<(Buffer, Submission), String> {
        let raw = unsafe { &*source.frame.as_ptr() };
        if raw.format != sys::AVPixelFormat::AV_PIX_FMT_VAAPI as i32 || raw.hw_frames_ctx.is_null()
        {
            return Err("Intel video source is not a VA-API surface".into());
        }
        let context = unsafe { &*(*raw.hw_frames_ctx).data.cast::<sys::AVHWFramesContext>() };
        let bytes_per_component = match context.sw_format {
            sys::AVPixelFormat::AV_PIX_FMT_NV12 => 1,
            sys::AVPixelFormat::AV_PIX_FMT_P010LE => 2,
            format => return Err(format!("Unsupported Intel video surface {format:?}")),
        };
        let mut derived = ptr::null_mut();
        let result = unsafe {
            sys::av_hwframe_ctx_create_derived(
                &mut derived,
                sys::AVPixelFormat::AV_PIX_FMT_VULKAN,
                self.device.vulkan,
                raw.hw_frames_ctx,
                sys::AV_HWFRAME_MAP_READ as i32 | sys::AV_HWFRAME_MAP_DIRECT as i32,
            )
        };
        if result < 0 {
            return Err(format!(
                "Derive Vulkan video surfaces: {}",
                ffmpeg::Error::from(result)
            ));
        }
        let mut mapped = ffmpeg::frame::Video::empty();
        unsafe {
            (*mapped.as_mut_ptr()).format = sys::AVPixelFormat::AV_PIX_FMT_VULKAN as i32;
            (*mapped.as_mut_ptr()).hw_frames_ctx = derived;
        }
        let result = unsafe {
            sys::av_hwframe_map(
                mapped.as_mut_ptr(),
                source.frame.as_ptr(),
                sys::AV_HWFRAME_MAP_READ as i32 | sys::AV_HWFRAME_MAP_DIRECT as i32,
            )
        };
        if result < 0 {
            return Err(format!(
                "Import Intel video to Vulkan without CPU copies: {}",
                ffmpeg::Error::from(result)
            ));
        }
        let width = raw.width as usize;
        let height = raw.height as usize;
        let pitch = width
            .checked_mul(bytes_per_component)
            .ok_or("Video pitch overflow")?;
        let y_size = pitch
            .checked_mul(height)
            .ok_or("Video plane size overflow")?;
        let uv_pitch = width.div_ceil(2) * 2 * bytes_per_component;
        let uv_size = uv_pitch
            .checked_mul(height.div_ceil(2))
            .ok_or("Video chroma size overflow")?;
        let planes = self.allocate(
            y_size
                .checked_add(uv_size)
                .ok_or("Video frame size overflow")?,
        )?;
        let command = self.command()?;
        let native = unsafe { &mut *(*mapped.as_mut_ptr()).data[0].cast::<sys::AVVkFrame>() };
        let frames = unsafe { (*mapped.as_ptr()).hw_frames_ctx };
        let frames = unsafe { (*frames).data.cast::<sys::AVHWFramesContext>() };
        let vulkan_frames = unsafe { &*(*frames).hwctx.cast::<sys::AVVulkanFramesContext>() };
        if let Some(lock) = vulkan_frames.lock_frame {
            unsafe {
                lock(frames, native);
            }
        }
        let mut semaphores = Vec::new();
        let mut wait_values = Vec::new();
        let mut signal_values = Vec::new();
        let image_count = native
            .img
            .iter()
            .take_while(|image| !image.is_null())
            .count();
        let mut barriers = Vec::new();
        for index in 0..image_count {
            barriers.push(
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::from_raw(native.access[index]))
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::from_raw(native.layout[index] as i32))
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(native.queue_family[index])
                    .dst_queue_family_index(
                        if native.queue_family[index] == vk::QUEUE_FAMILY_IGNORED {
                            vk::QUEUE_FAMILY_IGNORED
                        } else {
                            self.device.family
                        },
                    )
                    .image(vk::Image::from_raw(native.img[index] as u64))
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    ),
            );
            if !native.sem[index].is_null() {
                semaphores.push(vk::Semaphore::from_raw(native.sem[index] as u64));
                wait_values.push(native.sem_value[index]);
                signal_values.push(
                    native.sem_value[index]
                        .checked_add(1)
                        .expect("Vulkan video semaphore overflow"),
                );
            }
        }
        unsafe {
            self.device.raw.cmd_pipeline_barrier(
                command,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &barriers,
            );
        }
        for plane in 0..2 {
            let index = if image_count == 1 { 0 } else { plane };
            let aspect = if image_count == 1 {
                if plane == 0 {
                    vk::ImageAspectFlags::PLANE_0
                } else {
                    vk::ImageAspectFlags::PLANE_1
                }
            } else {
                vk::ImageAspectFlags::COLOR
            };
            let copy = vk::BufferImageCopy::default()
                .buffer_offset(if plane == 0 { 0 } else { y_size as u64 })
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(aspect)
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: if plane == 0 { width } else { width.div_ceil(2) } as u32,
                    height: if plane == 0 {
                        height
                    } else {
                        height.div_ceil(2)
                    } as u32,
                    depth: 1,
                });
            unsafe {
                self.device.raw.cmd_copy_image_to_buffer(
                    command,
                    vk::Image::from_raw(native.img[index] as u64),
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    planes.raw(),
                    &[copy],
                );
            }
        }
        unsafe { self.device.raw.end_command_buffer(command) }
            .map_err(|e| format!("Finish Intel video import: {e:?}"))?;
        let fence = unsafe {
            self.device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| format!("Create video import fence: {e:?}"))?;
        let stages = vec![vk::PipelineStageFlags::TRANSFER; semaphores.len()];
        let mut timeline = vk::TimelineSemaphoreSubmitInfo::default()
            .wait_semaphore_values(&wait_values)
            .signal_semaphore_values(&signal_values);
        let commands = [command];
        let submit = vk::SubmitInfo::default()
            .command_buffers(&commands)
            .wait_semaphores(&semaphores)
            .wait_dst_stage_mask(&stages)
            .signal_semaphores(&semaphores)
            .push_next(&mut timeline);
        let result = {
            let _guard = self
                .device
                .submission
                .lock()
                .expect("Vulkan submission mutex poisoned");
            unsafe {
                self.device
                    .raw
                    .queue_submit(self.device.queue, &[submit], fence)
            }
        };
        if result.is_ok() {
            for index in 0..image_count {
                native.layout[index] = vk::ImageLayout::TRANSFER_SRC_OPTIMAL.as_raw() as _;
                native.access[index] = vk::AccessFlags::TRANSFER_READ.as_raw();
                if native.queue_family[index] != vk::QUEUE_FAMILY_IGNORED {
                    native.queue_family[index] = self.device.family;
                }
                if !native.sem[index].is_null() {
                    native.sem_value[index] = native.sem_value[index]
                        .checked_add(1)
                        .expect("Vulkan semaphore overflow");
                }
            }
        }
        if let Some(unlock) = vulkan_frames.unlock_frame {
            unsafe {
                unlock(frames, native);
            }
        }
        result.map_err(|e| format!("Submit Intel video import: {e:?}"))?;
        let submission = Submission {
            pool: self.pool.clone(),
            command,
            fence,
            descriptor: None,
            _resources: vec![planes.clone()],
            _video: Some(mapped),
            _prior: Vec::new(),
            _images: Vec::new(),
        };

        let (planes, pitch, uv_pitch, y_size, prior) = if bytes_per_component == 2 {
            let output = self.allocate(planes.len() / 2)?;
            let mut arguments = self.arguments("p010_to_nv12")?;
            arguments
                .set("input", &planes.address().to_ne_bytes())?
                .set("output", &output.address().to_ne_bytes())?
                .set("count", &(output.len() as u64).to_ne_bytes())?;
            let mut converted = unsafe {
                self.dispatch(
                    arguments,
                    vec![planes, output.clone()],
                    [output.len(), 1, 1],
                )
            }?;
            converted._prior.push(submission);
            (output, pitch / 2, uv_pitch / 2, y_size / 2, converted)
        } else {
            (planes, pitch, uv_pitch, y_size, submission)
        };

        let count = width
            .checked_mul(height)
            .ok_or("Video pixel count overflow")?;
        let output = self.allocate(count.checked_mul(4).ok_or("Video RGBA size overflow")?)?;
        let mut parameters: shrimply_render_core::Nv12LayerParams = unsafe { std::mem::zeroed() };
        parameters.source_width = width as u32;
        parameters.source_height = height as u32;
        parameters.canvas_width = width as u32;
        parameters.y_plane = planes.address() as usize as _;
        parameters.uv_plane = (planes.address() + y_size as u64) as usize as _;
        parameters.y_pitch = pitch;
        parameters.uv_pitch = uv_pitch;
        parameters.inverse = shrimply_render_core::math::Mat3::IDENTITY;
        parameters.opacity = 1.0;
        parameters.kind = shrimply_render_core::LayerKind::Nv12;
        parameters.sample_method = shrimply_render_core::VideoSampleMethod::Nearest;
        let bytes = unsafe {
            std::slice::from_raw_parts(
                (&parameters as *const shrimply_render_core::Nv12LayerParams).cast(),
                size_of_val(&parameters),
            )
        };
        let layer = self.upload(bytes)?;
        let mut arguments = self.arguments("composite_nv12_layers")?;
        arguments
            .set("layers", &layer.address().to_ne_bytes())?
            .set("layer_count", &1_u64.to_ne_bytes())?
            .set("transforms", &0_u64.to_ne_bytes())?
            .set("transform_count", &0_u64.to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?
            .set("output_count", &(count as u64).to_ne_bytes())?
            .set("background", &0_u32.to_ne_bytes())?;
        let mut converted = unsafe {
            self.dispatch(
                arguments,
                vec![planes, layer, output.clone()],
                [count, 1, 1],
            )
        }?;
        converted._prior.push(prior);
        Ok((output, converted))
    }
}
