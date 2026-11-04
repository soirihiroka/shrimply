use crate::{Buffer, Renderer, Submission, device::Device};
use ash::{vk, vk::Handle};
use ffmpeg_next::{self as ffmpeg, sys};
use std::{ptr, sync::Arc};

/// Keeps the decoder surface alive until the final GPU read completes.
pub struct VideoFrame {
    pub frame: ffmpeg::frame::Video,
}

struct FrameLock {
    context: *mut sys::AVHWFramesContext,
    frame: *mut sys::AVVkFrame,
    unlock: Option<unsafe extern "C" fn(*mut sys::AVHWFramesContext, *mut sys::AVVkFrame)>,
}

impl Drop for FrameLock {
    fn drop(&mut self) {
        if let Some(unlock) = self.unlock {
            unsafe {
                unlock(self.context, self.frame);
            }
        }
    }
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

#[derive(Clone, Copy)]
struct Layout {
    width: usize,
    height: usize,
    component_bytes: usize,
    pitch: usize,
    uv_pitch: usize,
    y_bytes: usize,
    y_size: usize,
    uv_size: usize,
    length: usize,
}

fn frame_layout(source: &ffmpeg::frame::Video) -> Result<Layout, String> {
    let raw = unsafe { &*source.as_ptr() };
    if raw.format != sys::AVPixelFormat::AV_PIX_FMT_VAAPI as i32 || raw.hw_frames_ctx.is_null() {
        return Err("Intel video source is not a VA-API surface".into());
    }
    if raw.width <= 0 || raw.height <= 0 {
        return Err("Intel video surface dimensions must be positive".into());
    }
    let context = unsafe { &*(*raw.hw_frames_ctx).data.cast::<sys::AVHWFramesContext>() };
    let component_bytes = match context.sw_format {
        sys::AVPixelFormat::AV_PIX_FMT_NV12 => 1,
        sys::AVPixelFormat::AV_PIX_FMT_P010LE => 2,
        format => return Err(format!("Unsupported Intel video surface {format:?}")),
    };
    let width = raw.width as usize;
    let height = raw.height as usize;
    let pitch = width
        .checked_mul(component_bytes)
        .ok_or("Video pitch overflow")?;
    let y_bytes = pitch
        .checked_mul(height)
        .ok_or("Video plane size overflow")?;
    let y_size = y_bytes.next_multiple_of(size_of::<u32>());
    let uv_pitch = width.div_ceil(2) * 2 * component_bytes;
    let uv_size = uv_pitch
        .checked_mul(height.div_ceil(2))
        .ok_or("Video chroma size overflow")?;
    let length = y_size
        .checked_add(uv_size)
        .ok_or("Video frame size overflow")?;
    Ok(Layout {
        width,
        height,
        component_bytes,
        pitch,
        uv_pitch,
        y_bytes,
        y_size,
        uv_size,
        length,
    })
}

fn map(
    device: &Device,
    source: &ffmpeg::frame::Video,
    flags: i32,
) -> Result<ffmpeg::frame::Video, String> {
    let mut derived = ptr::null_mut();
    let result = unsafe {
        sys::av_hwframe_ctx_create_derived(
            &mut derived,
            sys::AVPixelFormat::AV_PIX_FMT_VULKAN,
            device.vulkan,
            (*source.as_ptr()).hw_frames_ctx,
            flags,
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
    // Export explicitly to DRM PRIME first. With MAP_DIRECT, FFmpeg's
    // VA-API mapper rejects a Vulkan destination before trying its mapper.
    let mut drm = ffmpeg::frame::Video::empty();
    unsafe {
        (*drm.as_mut_ptr()).format = sys::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
    }
    let result = unsafe { sys::av_hwframe_map(drm.as_mut_ptr(), source.as_ptr(), flags) };
    if result < 0 {
        return Err(format!(
            "Export Intel video as DMA-BUF: {}",
            ffmpeg::Error::from(result)
        ));
    }
    let result = unsafe { sys::av_hwframe_map(mapped.as_mut_ptr(), drm.as_ptr(), flags) };
    if result < 0 {
        return Err(format!(
            "Import Intel video to Vulkan without CPU copies: {}",
            ffmpeg::Error::from(result)
        ));
    }
    Ok(mapped)
}

impl Renderer {
    fn transfer_video(
        &self,
        source: &ffmpeg::frame::Video,
        planes: Buffer,
        layout: Layout,
        write: bool,
    ) -> Result<Submission, String> {
        let flags = sys::AV_HWFRAME_MAP_DIRECT as i32
            | if write {
                sys::AV_HWFRAME_MAP_WRITE as i32
            } else {
                sys::AV_HWFRAME_MAP_READ as i32
            };
        let mut mapped = map(&self.device, source, flags)?;
        let Layout {
            width,
            height,
            y_size,
            ..
        } = layout;
        let image_layout = if write {
            vk::ImageLayout::TRANSFER_DST_OPTIMAL
        } else {
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL
        };
        let access = if write {
            vk::AccessFlags2::TRANSFER_WRITE
        } else {
            vk::AccessFlags2::TRANSFER_READ
        };
        let command = self.command()?;
        let mut submission = Submission {
            pool: self.pool.clone(),
            command,
            fence: vk::Fence::null(),
            submitted: false,
            _resources: vec![planes.clone()],
            _video: None,
            _prior: Vec::new(),
            _images: Vec::new(),
            _textures: Vec::new(),
        };
        let native = unsafe { &mut *(*mapped.as_mut_ptr()).data[0].cast::<sys::AVVkFrame>() };
        let frames = unsafe { (*mapped.as_ptr()).hw_frames_ctx };
        let frames = unsafe { (*frames).data.cast::<sys::AVHWFramesContext>() };
        let vulkan_frames = unsafe { &*(*frames).hwctx.cast::<sys::AVVulkanFramesContext>() };
        if let Some(lock) = vulkan_frames.lock_frame {
            unsafe {
                lock(frames, native);
            }
        }
        let lock = FrameLock {
            context: frames,
            frame: native,
            unlock: vulkan_frames.unlock_frame,
        };
        let mut semaphores = Vec::new();
        let mut wait_values = Vec::new();
        let mut signal_values = Vec::new();
        let image_count = native.img.iter().take_while(|image| **image != 0).count();
        if !matches!(image_count, 1 | 2) {
            return Err(format!(
                "Expected one multiplane or two single-plane Intel video images, received {image_count}"
            ));
        }
        let mut barriers = Vec::new();
        for index in 0..image_count {
            barriers.push(
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .src_access_mask(vk::AccessFlags2::from_raw(native.access[index] as _))
                    .dst_access_mask(access)
                    .old_layout(vk::ImageLayout::from_raw(native.layout[index]))
                    .new_layout(image_layout)
                    .src_queue_family_index(native.queue_family[index])
                    .dst_queue_family_index(
                        if native.queue_family[index] == vk::QUEUE_FAMILY_IGNORED {
                            vk::QUEUE_FAMILY_IGNORED
                        } else {
                            self.device.family
                        },
                    )
                    .image(vk::Image::from_raw(native.img[index] as _))
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    ),
            );
            if native.sem[index] != 0 {
                semaphores.push(vk::Semaphore::from_raw(native.sem[index] as _));
                wait_values.push(native.sem_value[index]);
                signal_values.push(
                    native.sem_value[index]
                        .checked_add(1)
                        .expect("Vulkan video semaphore overflow"),
                );
            }
        }
        unsafe {
            self.device.raw.cmd_pipeline_barrier2(
                command,
                &vk::DependencyInfo::default()
                    .image_memory_barriers(&barriers)
                    .memory_barriers(&[vk::MemoryBarrier2::default()
                        .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                        .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                        .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                        .dst_access_mask(
                            vk::AccessFlags2::TRANSFER_READ | vk::AccessFlags2::TRANSFER_WRITE,
                        )]),
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
                let image = vk::Image::from_raw(native.img[index] as _);
                if write {
                    self.device.raw.cmd_copy_buffer_to_image(
                        command,
                        planes.raw(),
                        image,
                        image_layout,
                        &[copy],
                    );
                } else {
                    self.device.raw.cmd_copy_image_to_buffer(
                        command,
                        image,
                        image_layout,
                        planes.raw(),
                        &[copy],
                    );
                }
            }
        }
        let releases = (0..image_count)
            .map(|index| {
                let ignored = native.queue_family[index] == vk::QUEUE_FAMILY_IGNORED;
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                    .dst_stage_mask(vk::PipelineStageFlags2::NONE)
                    .src_access_mask(access)
                    .old_layout(image_layout)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(if ignored {
                        vk::QUEUE_FAMILY_IGNORED
                    } else {
                        self.device.family
                    })
                    .dst_queue_family_index(if ignored {
                        vk::QUEUE_FAMILY_IGNORED
                    } else {
                        vk::QUEUE_FAMILY_EXTERNAL
                    })
                    .image(vk::Image::from_raw(native.img[index] as _))
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(1)
                            .layer_count(1),
                    )
            })
            .collect::<Vec<_>>();
        unsafe {
            self.device.raw.cmd_pipeline_barrier2(
                command,
                &vk::DependencyInfo::default().image_memory_barriers(&releases),
            );
        }
        unsafe { self.device.raw.end_command_buffer(command) }
            .map_err(|e| format!("Finish Intel video import: {e:?}"))?;
        let fence = unsafe {
            self.device
                .raw
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| format!("Create video import fence: {e:?}"))?;
        submission.fence = fence;
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
            let _guard = self.device.queue_lock();
            unsafe {
                self.device
                    .raw
                    .queue_submit(self.device.queue, &[submit], fence)
            }
        };
        if result.is_ok() {
            submission.submitted = true;
            for index in 0..image_count {
                native.layout[index] = vk::ImageLayout::GENERAL.as_raw() as _;
                native.access[index] = 0;
                if native.queue_family[index] != vk::QUEUE_FAMILY_IGNORED {
                    native.queue_family[index] = vk::QUEUE_FAMILY_EXTERNAL;
                }
                if native.sem[index] != 0 {
                    native.sem_value[index] = native.sem_value[index]
                        .checked_add(1)
                        .expect("Vulkan semaphore overflow");
                }
            }
        }
        drop(lock);
        result.map_err(|e| format!("Submit Intel video import: {e:?}"))?;
        submission._video = Some(mapped);

        Ok(submission)
    }

    /// Import the VA-API surface through DRM PRIME; pixels stay on the GPU.
    pub fn video(&mut self, source: &Arc<VideoFrame>) -> Result<(Buffer, Submission), String> {
        let layout = frame_layout(&source.frame)?;
        let Layout {
            width,
            height,
            pitch,
            uv_pitch,
            y_bytes,
            y_size,
            uv_size,
            component_bytes: bytes_per_component,
            ..
        } = layout;
        let planes = self.allocate(layout.length)?;
        let submission = self.transfer_video(&source.frame, planes.clone(), layout, false)?;
        let (planes, pitch, uv_pitch, y_size, prior) = if bytes_per_component == 2 {
            let output = self.allocate((y_bytes + uv_size) / bytes_per_component)?;
            let mut arguments = self.arguments("p010_to_nv12")?;
            arguments
                .set("input", &planes.address().to_ne_bytes())?
                .set("output", &output.address().to_ne_bytes())?
                .set("count", &(output.len() as u64).to_ne_bytes())?
                .set(
                    "y_count",
                    &((y_bytes / bytes_per_component) as u64).to_ne_bytes(),
                )?
                .set(
                    "uv_offset",
                    &((y_size / bytes_per_component) as u64).to_ne_bytes(),
                )?;
            let mut converted = unsafe {
                self.dispatch(
                    arguments,
                    vec![planes, output.clone()],
                    [output.len(), 1, 1],
                )
            }?;
            converted._prior.push(submission);
            (output, pitch / 2, uv_pitch / 2, y_bytes / 2, converted)
        } else {
            (planes, pitch, uv_pitch, y_size, submission)
        };

        let count = width
            .checked_mul(height)
            .ok_or("Video pixel count overflow")?;
        let output = self.allocate(count.checked_mul(4).ok_or("Video RGBA size overflow")?)?;
        let mut arguments = self.arguments("nv12_to_rgba")?;
        arguments
            .set("y_plane", &planes.address().to_ne_bytes())?
            .set(
                "uv_plane",
                &(planes.address() + y_size as u64).to_ne_bytes(),
            )?
            .set("y_pitch", &(pitch as u64).to_ne_bytes())?
            .set("uv_pitch", &(uv_pitch as u64).to_ne_bytes())?
            .set("width", &(width as u32).to_ne_bytes())?
            .set("height", &(height as u32).to_ne_bytes())?
            .set("output", &output.address().to_ne_bytes())?;
        let mut converted =
            unsafe { self.dispatch(arguments, vec![planes, output.clone()], [count, 1, 1]) }?;
        converted._prior.push(prior);
        Ok((output, converted))
    }

    /// Convert RGBA and write an encoder's VA-API surface entirely on the GPU.
    pub fn write_video(
        &mut self,
        rgba: &Buffer,
        destination: &ffmpeg::frame::Video,
    ) -> Result<Submission, String> {
        let layout = frame_layout(destination)?;
        let Layout {
            width,
            height,
            pitch,
            uv_pitch,
            y_size,
            component_bytes,
            ..
        } = layout;
        let count = width
            .checked_mul(height)
            .ok_or("Encoder dimensions overflow")?;
        if count.checked_mul(size_of::<u32>()) != Some(rgba.len()) {
            return Err("RGBA buffer does not match the VA-API encoder surface dimensions".into());
        }
        let planes = self.allocate(layout.length)?;
        let mut prior = Vec::new();
        for chroma in [false, true] {
            let name = match (component_bytes, chroma) {
                (1, false) => "rgba_to_nv12_luma",
                (1, true) => "rgba_to_nv12_chroma",
                (2, false) => "rgba_to_p010_luma",
                (2, true) => "rgba_to_p010_chroma",
                _ => unreachable!("validated VA-API component size"),
            };
            let mut arguments = self.arguments(name)?;
            arguments
                .set("rgba", &rgba.address().to_ne_bytes())?
                .set("width", &(width as u32).to_ne_bytes())?
                .set("height", &(height as u32).to_ne_bytes())?
                .set(
                    if chroma { "uv_plane" } else { "y_plane" },
                    &(planes.address() + if chroma { y_size as u64 } else { 0 }).to_ne_bytes(),
                )?
                .set(
                    if chroma { "uv_pitch" } else { "y_pitch" },
                    &((if chroma { uv_pitch } else { pitch }) as u64).to_ne_bytes(),
                )?;
            let count = if chroma {
                width.div_ceil(2) * height.div_ceil(2)
            } else {
                count
            };
            prior.push(unsafe {
                self.dispatch(arguments, vec![rgba.clone(), planes.clone()], [count, 1, 1])
            }?);
        }
        let mut submission = self.transfer_video(destination, planes, layout, true)?;
        submission._prior = prior;
        Ok(submission)
    }
}
