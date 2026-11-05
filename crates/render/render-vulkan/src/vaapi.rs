use crate::{Buffer, Renderer, Submission, device::Device};
use ash::{vk, vk::Handle};
use ffmpeg_next::{self as ffmpeg, sys};
use std::{ptr, rc::Rc, sync::Arc};

const HD_MINIMUM_HEIGHT: u32 = 720;

pub fn video_color_space(frame: &ffmpeg::frame::Video) -> Result<ffmpeg::color::Space, String> {
    use ffmpeg::color::Space;
    match frame.color_space() {
        Space::Unspecified if frame.height() >= HD_MINIMUM_HEIGHT => Ok(Space::BT709),
        Space::Unspecified => Ok(Space::SMPTE170M),
        space @ (Space::BT709
        | Space::BT470BG
        | Space::SMPTE170M
        | Space::FCC
        | Space::SMPTE240M) => Ok(space),
        space => Err(format!("Unsupported Intel video color matrix: {space:?}")),
    }
}

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

pub(super) struct FramesContext(*mut sys::AVBufferRef);

impl Drop for FramesContext {
    fn drop(&mut self) {
        unsafe {
            sys::av_buffer_unref(&mut self.0);
        }
    }
}

pub(super) struct MappedVideo {
    frame: ffmpeg::frame::Video,
    // Must follow frame: unmap before releasing the owning context.
    _context: Rc<FramesContext>,
}

unsafe extern "C" fn release_borrowed_context(_: *mut std::ffi::c_void, _: *mut u8) {}

impl Renderer {
    fn map_video(
        &mut self,
        source: &ffmpeg::frame::Video,
        flags: i32,
    ) -> Result<MappedVideo, String> {
        let source_context = unsafe { (*source.as_ptr()).hw_frames_ctx };
        let key = (unsafe { (*source_context).data } as usize, flags);
        let context = if let Some(index) = self
            .video_contexts
            .iter()
            .position(|(candidate, _)| *candidate == key)
        {
            self.video_contexts
                .remove(index)
                .expect("cached video context")
                .1
        } else {
            let mut derived = ptr::null_mut();
            let result = unsafe {
                sys::av_hwframe_ctx_create_derived(
                    &mut derived,
                    sys::AVPixelFormat::AV_PIX_FMT_VULKAN,
                    self.device.vulkan,
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
            Rc::new(FramesContext(derived))
        };
        self.video_contexts.push_back((key, context.clone()));
        while self.video_contexts.len() > self.video_context_limit {
            self.video_contexts.pop_front();
        }
        // FFmpeg's execution pool retains mapped buffers until its next use. An owning
        // context reference in those buffers would form a cycle. This borrowed handle
        // identifies the native context; MappedVideo separately retains its real owner.
        let borrowed = unsafe {
            sys::av_buffer_create(
                (*context.0).data,
                (*context.0).size,
                Some(release_borrowed_context),
                ptr::null_mut(),
                0,
            )
        };
        if borrowed.is_null() {
            return Err("Retain Vulkan mapping context".into());
        }
        let mut mapped = ffmpeg::frame::Video::empty();
        unsafe {
            (*mapped.as_mut_ptr()).format = sys::AVPixelFormat::AV_PIX_FMT_VULKAN as i32;
            (*mapped.as_mut_ptr()).hw_frames_ctx = borrowed;
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
        Ok(MappedVideo {
            frame: mapped,
            _context: context,
        })
    }

    fn transfer_video(
        &mut self,
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
        let mut mapped = self.map_video(source, flags)?;
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
        let native = unsafe { &mut *(*mapped.frame.as_mut_ptr()).data[0].cast::<sys::AVVkFrame>() };
        let frames = unsafe { (*mapped.frame.as_ptr()).hw_frames_ctx };
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
        let mut waits = Vec::new();
        let mut signals = Vec::new();
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
                let semaphore = vk::Semaphore::from_raw(native.sem[index] as _);
                waits.push((semaphore, native.sem_value[index]));
                signals.push((
                    semaphore,
                    native.sem_value[index]
                        .checked_add(1)
                        .expect("Vulkan video semaphore overflow"),
                ));
            }
        }
        let command = self.command()?;
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
        let mut submission =
            self.submit_with_semaphores(command, vec![planes], &waits, &signals)?;
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
        drop(lock);
        submission._video = Some(mapped);

        Ok(submission)
    }

    /// Import the VA-API surface through DRM PRIME; pixels stay on the GPU.
    pub fn video(&mut self, source: &Arc<VideoFrame>) -> Result<(Buffer, Submission), String> {
        use ffmpeg::color::Range;
        let colorspace = video_color_space(&source.frame)?;
        let coefficients = unsafe {
            std::slice::from_raw_parts(
                sys::sws_getCoefficients(sys::AVColorSpace::from(colorspace) as i32),
                4,
            )
        };
        let coefficients = coefficients
            .iter()
            .flat_map(|value| value.to_ne_bytes())
            .collect::<Vec<_>>();
        let mut chroma_x = 0;
        let mut chroma_y = 0;
        let location = unsafe { (*source.frame.as_ptr()).chroma_location };
        let location = if location == sys::AVChromaLocation::AVCHROMA_LOC_UNSPECIFIED {
            sys::AVChromaLocation::AVCHROMA_LOC_LEFT
        } else {
            location
        };
        let result =
            unsafe { sys::av_chroma_location_enum_to_pos(&mut chroma_x, &mut chroma_y, location) };
        if result < 0 {
            return Err(format!(
                "Unsupported Intel video chroma location: {location:?}"
            ));
        }
        // FFmpeg specifies chroma positions in units of 1/256 luma pixels.
        const CHROMA_POSITION_SCALE: f32 = 256.0;
        let chroma_offset = [chroma_x, chroma_y]
            .map(|position| (position as f32 / CHROMA_POSITION_SCALE).to_ne_bytes());
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
            .set("output", &output.address().to_ne_bytes())?
            .set("coefficients", &coefficients)?
            .set("chroma_offset", chroma_offset.as_flattened())?
            .set(
                "full_range",
                &[u8::from(source.frame.color_range() == Range::JPEG)],
            )?;
        let mut converted =
            unsafe { self.dispatch(arguments, vec![planes, output.clone()], [width, height, 1]) }?;
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
            let extent = if chroma {
                [width.div_ceil(2), height.div_ceil(2), 1]
            } else {
                [width, height, 1]
            };
            prior.push(unsafe {
                self.dispatch(arguments, vec![rgba.clone(), planes.clone()], extent)
            }?);
        }
        let mut submission = self.transfer_video(destination, planes, layout, true)?;
        submission._prior = prior;
        Ok(submission)
    }
}
