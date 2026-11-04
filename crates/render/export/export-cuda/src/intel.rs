use super::*;
use shrimply_preview_render_vulkan::{Compositor, Rendered};

const MAX_QUANTIZATION_PARAMETER: u32 = 51;

fn unmodified_video(
    plan: &shrimply_preview_render_core::FramePlan,
    format: sys::AVPixelFormat,
) -> Result<Option<ffmpeg::frame::Video>, String> {
    use shrimply_preview_render_core::Source;
    use shrimply_render_core::{LayerBlendMode, VideoSampleMethod, math::Mat3};
    let [layer] = plan.layers.as_slice() else {
        return Ok(None);
    };
    let Source::Video(video) = &layer.source else {
        return Ok(None);
    };
    let parameters = layer.parameters;
    if !plan.external_layers.is_empty()
        || !layer.effects.is_empty()
        || !layer.transitions.is_empty()
        || layer.motion_blur.is_some()
        || layer.alpha_mask.is_some()
        || layer.video_mask.is_some()
        || layer.stabilization.is_some()
        || layer.morph_scene.is_some()
        || layer.render_size != (plan.width, plan.height)
        || layer.transform != Mat3::IDENTITY
        || layer.output_transform != Mat3::IDENTITY
        || parameters.opacity != 1.0
        || parameters.crop != [0.0; 4]
        || parameters.padding != [0.0; 4]
        || parameters.motion_sample_count != 0
        || parameters.motion_transform_count != 0
        || (parameters.source_width, parameters.source_height) != (plan.width, plan.height)
        || !matches!(
            parameters.blend_mode,
            LayerBlendMode::Normal | LayerBlendMode::PassThrough
        )
        || !matches!(
            parameters.sample_method,
            VideoSampleMethod::Nearest | VideoSampleMethod::Bilinear | VideoSampleMethod::Bicubic
        )
        || (video.frame.width(), video.frame.height()) != (plan.width, plan.height)
        || video.frame.color_range() == ffmpeg::color::Range::JPEG
        || shrimply_render_vulkan::video_color_space(&video.frame)? != ffmpeg::color::Space::BT709
    {
        return Ok(None);
    }
    let native = unsafe { &*video.frame.as_ptr() };
    if native.hw_frames_ctx.is_null()
        || native.crop_left != 0
        || native.crop_right != 0
        || native.crop_top != 0
        || native.crop_bottom != 0
        || native.flags & sys::AV_FRAME_FLAG_INTERLACED != 0
        || !matches!(
            native.chroma_location,
            sys::AVChromaLocation::AVCHROMA_LOC_UNSPECIFIED
                | sys::AVChromaLocation::AVCHROMA_LOC_LEFT
        )
    {
        return Ok(None);
    }
    let context = unsafe {
        &*(*native.hw_frames_ctx)
            .data
            .cast::<sys::AVHWFramesContext>()
    };
    if context.sw_format != format {
        return Ok(None);
    }
    // Video::clone performs a software pixel copy. Reference the native surface
    // instead, preserving its decoder context until the encoder releases it.
    let mut frame = ffmpeg::frame::Video::empty();
    ffmpeg_check(
        unsafe { sys::av_frame_ref(frame.as_mut_ptr(), video.frame.as_ptr()) },
        "Retain Intel decoder surface for export",
    )?;
    Ok(Some(frame))
}

pub(super) struct IntelBackend {
    settings: ExportSettings,
    compositor: Option<Compositor>,
    hardware_frames: Option<HwFrameContext>,
}

impl IntelBackend {
    pub fn new(settings: ExportSettings) -> Self {
        Self {
            settings,
            compositor: None,
            hardware_frames: None,
        }
    }

    fn render(
        &mut self,
        project: &Project,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<Rendered, String> {
        self.compositor
            .as_mut()
            .ok_or("Intel export renderer was not prepared")?
            .render_accurate(
                project,
                position,
                u32::from_le_bytes([0, 0, 0, self.settings.background_alpha]),
                cancelled,
            )
    }
}

impl VideoBackend for IntelBackend {
    fn name(&self) -> &'static str {
        "intel-vaapi"
    }

    fn encoder_label(&self, codec: ExportVideoCodec) -> &'static str {
        match codec {
            ExportVideoCodec::H264 => "h264_vaapi",
            ExportVideoCodec::H265 => "hevc_vaapi",
            ExportVideoCodec::Gif => "gif",
        }
    }

    fn validate(&self, project: &Project, settings: &core::ExportSettings) -> Result<(), String> {
        let size = project.canvas_size;
        if size.width == 0 || size.height == 0 {
            return Err("Intel export dimensions must be positive".into());
        }
        if settings.video_codec == ExportVideoCodec::Gif {
            return Ok(());
        }
        if !size.width.is_multiple_of(2) || !size.height.is_multiple_of(2) {
            return Err("VA-API export requires an even canvas width and height".into());
        }
        if self.settings.profile == ExportProfile::Main10
            && settings.video_codec == ExportVideoCodec::H264
        {
            return Err("Intel H.264 encoding does not support Main10; use H.265".into());
        }
        if self.settings.constant_qp > MAX_QUANTIZATION_PARAMETER {
            return Err("Intel encoder QP must be between 0 and 51".into());
        }
        if !matches!(
            self.settings.rate_control,
            ExportRateControl::ConstantQp
                | ExportRateControl::ConstantBitrate
                | ExportRateControl::VariableBitrate
        ) {
            return Err("Intel export supports constant QP, CBR, and VBR rate control".into());
        }
        if self.settings.rate_control != ExportRateControl::ConstantQp && settings.bitrate_kbps == 0
        {
            return Err("Intel bitrate-controlled export requires a positive bitrate".into());
        }
        if self.settings.rate_control == ExportRateControl::VariableBitrate
            && self.settings.max_bitrate_kbps < settings.bitrate_kbps
        {
            return Err("Intel VBR maximum bitrate must not be below the target bitrate".into());
        }
        Ok(())
    }

    fn prepare(
        &mut self,
        project: &Project,
        settings: &core::ExportSettings,
        _cancelled: &AtomicBool,
    ) -> Result<(), String> {
        let mut compositor = Compositor::new()?;
        compositor.set_decoder_limit(self.settings.maximum_temporal_decoders);
        self.compositor = Some(compositor);
        if settings.video_codec != ExportVideoCodec::Gif {
            let device = shrimply_render_vulkan::device::Device::get()?;
            let reference = unsafe { sys::av_buffer_ref(device.va) };
            if reference.is_null() {
                return Err("Retain Intel VA-API encoder device".into());
            }
            self.hardware_frames = Some(HwFrameContext::initialize(
                project,
                &self.settings,
                reference,
                sys::AVPixelFormat::AV_PIX_FMT_VAAPI,
            )?);
        }
        Ok(())
    }

    fn open_video_encoder(
        &mut self,
        project: &Project,
        settings: &core::ExportSettings,
        global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
        let hardware = self
            .hardware_frames
            .as_ref()
            .ok_or("Intel encoder frames were not prepared")?;
        let name = self.encoder_label(settings.video_codec);
        let codec = ffmpeg::codec::encoder::find_by_name(name)
            .ok_or_else(|| format!("FFmpeg encoder {name} is unavailable"))?;
        let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
            .encoder()
            .video()
            .map_err(|e| e.to_string())?;
        encoder.set_width(project.canvas_size.width);
        encoder.set_height(project.canvas_size.height);
        encoder.set_format(Pixel::VAAPI);
        encoder.set_time_base(video_time_base(settings.fps)?);
        encoder.set_frame_rate(Some(video_frame_rate(settings.fps)?));
        encoder.set_gop(video_gop(settings));
        encoder.set_max_b_frames(settings.b_frames as usize);
        if self.settings.rate_control != ExportRateControl::ConstantQp {
            encoder.set_bit_rate(settings.bitrate_kbps as usize * 1_000);
            encoder.set_max_bit_rate(
                if self.settings.rate_control == ExportRateControl::ConstantBitrate {
                    settings.bitrate_kbps
                } else {
                    self.settings.max_bitrate_kbps
                } as usize
                    * 1_000,
            );
        }
        unsafe {
            let context = encoder.as_mut_ptr();
            (*context).sw_pix_fmt = hardware.pixel_format().sw_format();
            (*context).hw_device_ctx = hardware.device_ref()?;
            (*context).hw_frames_ctx = hardware.encoder_ref()?;
            set_bt709_video_metadata(context);
            if global_header {
                (*context).flags |= sys::AV_CODEC_FLAG_GLOBAL_HEADER as i32;
            }
        }
        let mut options = ffmpeg::Dictionary::new();
        options.set("profile", profile_name(&self.settings));
        options.set(
            "rc_mode",
            match self.settings.rate_control {
                ExportRateControl::ConstantQp => "CQP",
                ExportRateControl::ConstantBitrate => "CBR",
                ExportRateControl::VariableBitrate => "VBR",
                _ => unreachable!("validated Intel rate control"),
            },
        );
        if self.settings.rate_control == ExportRateControl::ConstantQp {
            options.set("qp", &self.settings.constant_qp.to_string());
        }
        encoder
            .open_as_with(codec, options)
            .map_err(|e| format!("Open Intel {name} encoder: {e}"))
    }

    fn render_video_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let compositor = self
            .compositor
            .as_mut()
            .ok_or("Intel renderer was not prepared")?;
        let plan = compositor.prepare_accurate(project, position, cancelled)?;
        let hardware = self
            .hardware_frames
            .as_ref()
            .ok_or("Intel encoder frames were not prepared")?;
        // Like FFmpeg's passthrough filters, preserve the native decoded surface
        // when no rendering or color/format conversion is necessary.
        if let Some(mut frame) = unmodified_video(&plan, hardware.pixel_format().sw_format())? {
            // Let the encoder choose picture types for the requested GOP/B-frame settings.
            unsafe {
                (*frame.as_mut_ptr()).pict_type = sys::AVPictureType::AV_PICTURE_TYPE_NONE;
            }
            set_bt709_frame_metadata(&mut frame);
            return Ok(RenderedFrame {
                frame,
                timing: FrameTiming::default(),
            });
        }
        let rendered = compositor.render_gpu(
            plan,
            u32::from_le_bytes([0, 0, 0, self.settings.background_alpha]),
        )?;
        let mut frame = self
            .hardware_frames
            .as_ref()
            .ok_or("Intel encoder frames were not prepared")?
            .frame()?;
        let submission = self
            .compositor
            .as_mut()
            .expect("rendered Intel frame")
            .renderer
            .write_video(&rendered.buffer, &frame)?;
        // VA-API's encoder consumes this surface after the Vulkan release. This
        // fence waits for GPU work only; no pixels are downloaded or reuploaded.
        submission.wait()?;
        set_bt709_frame_metadata(&mut frame);
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming::default(),
        })
    }

    fn render_rgba_frame(
        &mut self,
        project: &Project,
        _settings: &core::ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        let rendered = self.render(project, position, cancelled)?;
        // GIF has a CPU encoder; only this explicit RGBA path requests pixels.
        let bytes = self
            .compositor
            .as_ref()
            .expect("rendered Intel frame")
            .renderer
            .readback(&rendered.buffer)?;
        let mut frame = ffmpeg::frame::Video::new(Pixel::RGBA, rendered.width, rendered.height);
        let stride = frame.stride(0);
        let row_bytes = rendered.width as usize * size_of::<u32>();
        for (destination, source) in frame
            .data_mut(0)
            .chunks_mut(stride)
            .zip(bytes.chunks_exact(row_bytes))
        {
            destination[..row_bytes].copy_from_slice(source);
        }
        Ok(RenderedFrame {
            frame,
            timing: FrameTiming::default(),
        })
    }

    fn shutdown(&mut self) {
        self.compositor.take();
        self.hardware_frames.take();
    }
}
