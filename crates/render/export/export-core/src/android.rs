//! MediaCodec surface adapter for the shared export pipeline and Vulkan compositor.
use crate::video::{self as core, ExportSettings, FrameTiming, RenderedFrame, VideoBackend};
use ffmpeg_next::{self as ffmpeg, format::Pixel, sys};
use ndk::native_window::NativeWindow;
use shrimply_preview_render_vulkan::{Compositor, Rendered};
use shrimply_project_document::project::{Project, Time};
use shrimply_render_vulkan::surface::Renderer;
use std::{
    ptr,
    sync::{Arc, atomic::AtomicBool},
};

pub struct Backend {
    maximum_temporal_decoders: usize,
    compositor: Option<Compositor>,
    surface: Option<Renderer>,
    hardware: Option<Hardware>,
}

struct Hardware(*mut sys::AVBufferRef);
impl Drop for Hardware {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.0) };
    }
}

impl Backend {
    pub fn new(maximum_temporal_decoders: usize) -> Self {
        Self {
            maximum_temporal_decoders,
            compositor: None,
            surface: None,
            hardware: None,
        }
    }

    fn render(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<Rendered, String> {
        self.compositor
            .as_mut()
            .ok_or("Android export renderer was not prepared")?
            .render_accurate(
                project,
                position,
                u32::from_le_bytes([0, 0, 0, settings.background_alpha]),
                cancelled,
            )
    }
}

impl VideoBackend for Backend {
    fn name(&self) -> &'static str {
        "android-vulkan-mediacodec"
    }

    fn encoder_label(&self, codec: core::ExportVideoCodec) -> &'static str {
        match codec {
            core::ExportVideoCodec::H264 => "h264_mediacodec",
            core::ExportVideoCodec::H265 => "hevc_mediacodec",
            core::ExportVideoCodec::Gif => "gif",
        }
    }

    fn validate(&self, _project: &Project, settings: &ExportSettings) -> Result<(), String> {
        if settings.video_codec != core::ExportVideoCodec::Gif {
            match settings.rate_control {
                core::ExportRateControl::ConstantQp
                    if settings.constant_qp > core::settings::MAX_CONSTANT_QP =>
                {
                    return Err("Constant QP must be between 0 and 51".into());
                }
                core::ExportRateControl::ConstantQp => {}
                core::ExportRateControl::ConstantBitrate
                | core::ExportRateControl::VariableBitrate
                    if settings.bitrate_kbps > 0 => {}
                core::ExportRateControl::ConstantBitrate
                | core::ExportRateControl::VariableBitrate => {
                    return Err("Video bitrate must be larger than zero".into());
                }
                _ => return Err("The selected rate control is not supported by MediaCodec".into()),
            }
        }
        if settings.b_frames != 0 {
            return Err("Android MediaCodec export requires B frames to be disabled for reliable decode timestamps".into());
        }
        if settings.video_codec != core::ExportVideoCodec::Gif
            && !shrimply_render_vulkan::device::Device::get()?.presentation_timing
        {
            return Err(
                "Hardware export requires VK_GOOGLE_display_timing to preserve media timestamps"
                    .into(),
            );
        }
        Ok(())
    }

    fn prepare(
        &mut self,
        _project: &Project,
        _settings: &ExportSettings,
        _cancelled: &AtomicBool,
    ) -> Result<(), String> {
        let mut compositor = Compositor::new()?;
        compositor.set_decoder_limit(self.maximum_temporal_decoders);
        self.compositor = Some(compositor);
        Ok(())
    }

    fn open_video_encoder(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
        let mut hardware = Hardware(ptr::null_mut());
        let mut options = ffmpeg::Dictionary::new();
        options.set("create_window", "1");
        let mut raw_options = unsafe { options.disown() };
        let result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut hardware.0,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_MEDIACODEC,
                ptr::null(),
                raw_options,
                0,
            )
        };
        unsafe { sys::av_dict_free(&mut raw_options) };
        core::ffmpeg_check(result, "Create MediaCodec encoder surface")?;
        let device = unsafe { &*(*hardware.0).data.cast::<sys::AVHWDeviceContext>() };
        let native = unsafe { &*device.hwctx.cast::<sys::AVMediaCodecDeviceContext>() };
        let window = std::ptr::NonNull::new(native.native_window.cast())
            .ok_or("MediaCodec did not create an encoder input surface")?;
        let window = unsafe { NativeWindow::clone_from_ptr(window) };
        window
            .set_buffers_geometry(
                project.canvas_size.width as i32,
                project.canvas_size.height as i32,
                None,
            )
            .map_err(|error| format!("Set encoder surface dimensions: {error}"))?;
        let name = self.encoder_label(settings.video_codec);
        let codec = ffmpeg::codec::encoder::find_by_name(name)
            .ok_or_else(|| format!("FFmpeg encoder {name} is unavailable"))?;
        let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|error| error.to_string())?;
        core::configure_video_encoder(
            &mut encoder,
            project,
            settings,
            Pixel::MEDIACODEC,
            global_header,
        )?;
        unsafe {
            (*encoder.as_mut_ptr()).hw_device_ctx = sys::av_buffer_ref(hardware.0);
            if (*encoder.as_ptr()).hw_device_ctx.is_null() {
                return Err("Retain MediaCodec encoder device".into());
            }
        }
        let mut options = ffmpeg::Dictionary::new();
        options.set("ndk_codec", "1");
        options.set(
            "bitrate_mode",
            if settings.rate_control == core::ExportRateControl::ConstantBitrate {
                "cbr"
            } else {
                "vbr"
            },
        );
        if settings.rate_control == core::ExportRateControl::ConstantQp {
            unsafe {
                (*encoder.as_mut_ptr()).qmin = settings.constant_qp as i32;
                (*encoder.as_mut_ptr()).qmax = settings.constant_qp as i32;
            }
        }
        let encoder = encoder
            .open_as_with(codec, options)
            .map_err(|error| format!("Open hardware encoder {name}: {error}"))?;
        self.surface = Some(Renderer::new(
            &window,
            project.canvas_size.width,
            project.canvas_size.height,
        ));
        self.hardware = Some(hardware);
        Ok(encoder)
    }

    fn render_video_frame(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let rendered = self.render(project, settings, position, cancelled)?;
        let (texture, submission) = self
            .compositor
            .as_mut()
            .expect("prepared export compositor")
            .renderer
            .presentation_texture(&rendered.buffer, rendered.width, rendered.height)?;
        submission.wait()?;
        // Android's Vulkan WSI forwards desiredPresentTime to the window's
        // buffer timestamp. Zero means automatic wall-clock timing, so use 1ns
        // for the first frame; MediaCodec rounds that to the intended 0us PTS.
        let timestamp = shrimply_math_core::time_ticks(position, 1_000_000_000)
            .ok_or("export timestamp exceeds the native range")?
            .max(1);
        let valid = self
            .surface
            .as_mut()
            .ok_or("MediaCodec input surface was not prepared")?
            .draw_texture(
                Some(&texture),
                Some(timestamp),
                |canvas, image, width, height| {
                    assert_eq!(
                        (width, height),
                        (rendered.width, rendered.height),
                        "encoder surface dimensions changed"
                    );
                    canvas.clear(skia_safe::Color::TRANSPARENT);
                    canvas.draw_image(image.expect("completed export texture"), (0, 0), None);
                },
            );
        if !valid {
            return Err("MediaCodec input surface changed during export".into());
        }
        // Pixels have already been queued directly on the encoder's GPU input
        // surface. FFmpeg consumes this reference-counted frame as a submission
        // marker, then drains MediaCodec's actual compressed output packets.
        let mut frame = ffmpeg::frame::Video::empty();
        frame.set_format(Pixel::MEDIACODEC);
        frame.set_width(rendered.width);
        frame.set_height(rendered.height);
        unsafe {
            let buffer = sys::av_buffer_alloc(1);
            if buffer.is_null() {
                return Err("Allocate encoder submission reference".into());
            }
            (*frame.as_mut_ptr()).buf[0] = buffer;
            (*frame.as_mut_ptr()).data[0] = (*buffer).data;
        }
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming::default(),
        })
    }

    fn render_rgba_frame(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let rendered = self.render(project, settings, position, cancelled)?;
        rendered.wait()?;
        // GIF uses the same explicit readback and shared palette encoder as the
        // desktop backends. H.264/H.265 never enter this CPU-pixel path.
        let bytes = self
            .compositor
            .as_ref()
            .expect("prepared export compositor")
            .renderer
            .readback(&rendered.buffer)?;
        let frame = core::rgba_frame(rendered.width, rendered.height, &bytes)?;
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming::default(),
        })
    }

    fn shutdown(&mut self) {
        self.surface.take();
        self.hardware.take();
        self.compositor.take();
    }
}

pub fn export_project<F>(
    project: Project,
    settings: ExportSettings,
    maximum_temporal_decoders: usize,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(core::ExportProgress),
{
    core::export_project(
        project,
        settings,
        Backend::new(maximum_temporal_decoders),
        cancelled,
        progress,
    )
}
