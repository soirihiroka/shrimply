//! MediaCodec surface adapter for the shared export pipeline and Vulkan compositor.
use crate::video::{
    self as core, ExportSettings, ExportVideoCodec, FrameTiming, RenderedFrame, VideoBackend,
};
use core::settings::VideoEncoder;
use ffmpeg_next::{
    self as ffmpeg,
    format::{Pixel, context::StreamIo},
    sys,
};
use ndk::native_window::NativeWindow;
use shrimply_preview_render_vulkan::{Compositor, Rendered};
use shrimply_project_document::project::{Project, Time};
use shrimply_render_vulkan::surface::Renderer;
use std::{
    fs::File,
    io::{self, Seek, Write},
    os::fd::AsRawFd,
    ptr,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Backend {
    maximum_temporal_decoders: usize,
    compositor: Option<Compositor>,
    surface: Option<Renderer>,
    buffer_format: Option<Pixel>,
    scaler: Option<ffmpeg::software::scaling::Context>,
}

struct Hardware(*mut sys::AVBufferRef);
impl Drop for Hardware {
    fn drop(&mut self) {
        unsafe { sys::av_buffer_unref(&mut self.0) };
    }
}

pub mod recording;

const MEDIA_CODEC_INPUTS: [Pixel; 3] = [Pixel::MEDIACODEC, Pixel::NV12, Pixel::YUV420P];

#[derive(serde::Deserialize)]
pub struct EncoderNames {
    hardware: [Option<String>; 2],
    software: [Option<String>; 2],
}

static ENCODER_NAMES: OnceLock<EncoderNames> = OnceLock::new();

pub fn set_encoder_names(names: EncoderNames) {
    ENCODER_NAMES.get_or_init(|| names);
}

fn encoder_name(encoder: VideoEncoder) -> Option<&'static str> {
    let names = ENCODER_NAMES.get()?;
    let format = encoder.format();
    let names = if format.hardware {
        &names.hardware
    } else {
        &names.software
    };
    match format.codec {
        ExportVideoCodec::H264 => names[0].as_deref(),
        ExportVideoCodec::H265 => names[1].as_deref(),
        ExportVideoCodec::Gif => None,
    }
}

fn media_inputs(encoder: VideoEncoder) -> impl Iterator<Item = Pixel> {
    let surface = encoder.format().hardware
        && shrimply_render_vulkan::device::Device::get()
            .is_ok_and(|device| device.presentation_timing);
    MEDIA_CODEC_INPUTS
        .into_iter()
        .filter(move |&pixel| surface || pixel != Pixel::MEDIACODEC)
}

pub fn settings_backend() -> &'static core::settings::Encoders {
    use core::settings::Encoders;
    static ENCODERS: OnceLock<Encoders> = OnceLock::new();
    ENCODERS.get_or_init(|| {
        Encoders::probe(|encoder| {
            let Some(name) = encoder_name(encoder) else {
                return false;
            };
            let wrapper = encoder.format().ffmpeg;
            // Some devices reject the Vulkan swapchain on hardware input windows.
            // Test buffered input as well before declaring the encoder unavailable.
            media_inputs(encoder).any(|pixel| {
                let attempt = core::probe_encoder(wrapper, pixel).and_then(|encoder| {
                    let mut options = ffmpeg::Dictionary::new();
                    set_input_options(&mut options, name);
                    options.set("bitrate_mode", "vbr");
                    open_encoder(encoder, options)
                });
                if let Err(error) = &attempt {
                    tracing::debug!(name, ?pixel, %error, "MediaCodec input probe failed");
                }
                attempt.is_ok()
            })
        })
    })
}

fn set_input_options(options: &mut ffmpeg::Dictionary<'static>, name: &str) {
    options.set("codec_name", name);
    options.set("ndk_codec", "1");
    options.set("pts_as_dts", "0");
}

fn encoder_window(
    encoder: &mut ffmpeg::codec::encoder::video::Video,
) -> Result<NativeWindow, String> {
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
        .set_buffers_geometry(encoder.width() as i32, encoder.height() as i32, None)
        .map_err(|error| format!("Set encoder surface dimensions: {error}"))?;
    // Move our sole device reference into the codec context. NativeWindow
    // also owns a reference and can outlive the encoder during teardown.
    unsafe {
        (*encoder.as_mut_ptr()).hw_device_ctx = std::mem::replace(&mut hardware.0, ptr::null_mut())
    };
    Ok(window)
}

fn open_encoder(
    mut encoder: ffmpeg::codec::encoder::video::Video,
    options: ffmpeg::Dictionary<'static>,
) -> Result<(ffmpeg::codec::encoder::video::Encoder, Option<Renderer>), String> {
    let name = encoder
        .codec()
        .ok_or("Missing MediaCodec encoder")?
        .name()
        .to_owned();
    let window = (encoder.format() == Pixel::MEDIACODEC)
        .then(|| encoder_window(&mut encoder))
        .transpose()?;
    let encoder = encoder.open_with(options).map_err(|error| match error {
        ffmpeg::Error::Other { errno } if errno == libc::ENOSYS => {
            format!("MediaCodec encoder {name} does not support the requested fixed QP")
        }
        _ => format!("Open MediaCodec encoder {name}: {error}"),
    })?;
    let dimensions = (encoder.width(), encoder.height());
    let surface = window
        .as_ref()
        .map(|window| {
            let surface = Renderer::try_new(window, dimensions.0, dimensions.1)?;
            if surface.size() != dimensions {
                return Err("MediaCodec surface does not match the export dimensions".into());
            }
            Ok::<_, String>(surface)
        })
        .transpose()?;
    Ok((encoder, surface))
}

fn open_project_encoder(
    project: &Project,
    settings: &ExportSettings,
    pixel: Pixel,
) -> Result<(ffmpeg::codec::encoder::video::Encoder, Option<Renderer>), String> {
    let name = settings.video_encoder.format().ffmpeg;
    let codec = ffmpeg::codec::encoder::find_by_name(name)
        .ok_or_else(|| format!("FFmpeg encoder {name} is unavailable"))?;
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|error| error.to_string())?;
    // FFmpeg probes global headers with dummy input followed by a flush.
    // MediaCodec encoders can stall after that flush. Mux the real first packet.
    core::configure_video_encoder(&mut encoder, project, settings, pixel, false)?;
    unsafe {
        if settings.b_frames > 0 {
            // Android otherwise defaults AVC to Baseline, which cannot
            // represent B frames even when the encoder supports them.
            if settings.video_encoder.codec() == ExportVideoCodec::H264 {
                (*encoder.as_mut_ptr()).profile = sys::AV_PROFILE_H264_MAIN;
            }
            (*encoder.as_mut_ptr()).strict_std_compliance = sys::FF_COMPLIANCE_EXPERIMENTAL;
        }
    }
    let mut options = ffmpeg::Dictionary::new();
    let name =
        encoder_name(settings.video_encoder).ok_or("No MediaCodec encoder for this format")?;
    set_input_options(&mut options, name);
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
    open_encoder(encoder, options)
}

impl Backend {
    pub fn new(maximum_temporal_decoders: usize) -> Self {
        Self {
            maximum_temporal_decoders,
            compositor: None,
            surface: None,
            buffer_format: None,
            scaler: None,
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

    fn decode_order_delay(&self) -> Option<u32> {
        // AVC and HEVC bound decoded-picture buffering to 16 pictures.
        // MediaCodec's max-bframes config is a GOP setting, not a DTS offset.
        Some(16)
    }

    fn validate(&self, _project: &Project, settings: &ExportSettings) -> Result<(), String> {
        core::settings::validate_rate_control(settings)
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
        _global_header: bool,
    ) -> Result<ffmpeg::codec::encoder::video::Encoder, String> {
        let mut attempt = settings.clone();
        let mut first_error = None;
        for b_frames in
            std::iter::once(settings.b_frames).chain((settings.b_frames != 0).then_some(0))
        {
            attempt.b_frames = b_frames;
            for pixel in media_inputs(settings.video_encoder) {
                match open_project_encoder(project, &attempt, pixel) {
                    Ok((encoder, surface)) => {
                        self.surface = surface;
                        self.buffer_format = (pixel != Pixel::MEDIACODEC).then_some(pixel);
                        if b_frames != settings.b_frames {
                            tracing::warn!(
                                requested = settings.b_frames,
                                "MediaCodec opened without B-frames"
                            );
                        }
                        return Ok(encoder);
                    }
                    Err(error) => {
                        tracing::debug!(?pixel, b_frames, %error, "MediaCodec export input failed");
                        first_error.get_or_insert(error);
                    }
                }
            }
        }
        Err(first_error.expect("at least one MediaCodec input"))
    }

    fn render_video_frame(
        &mut self,
        project: &Project,
        settings: &ExportSettings,
        position: Time,
        cancelled: &AtomicBool,
    ) -> Result<RenderedFrame, String> {
        if let Some(pixel) = self.buffer_format {
            let mut rendered = self.render_rgba_frame(project, settings, position, cancelled)?;
            rendered.frame = core::rgba_to_bt709(&mut self.scaler, &rendered.frame, pixel)?;
            return Ok(rendered);
        }
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
        // Buffered MediaCodec, software video and GIF share this readback.
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
        self.compositor.take();
    }
}

pub fn export_project<F>(
    project: Project,
    settings: ExportSettings,
    maximum_temporal_decoders: usize,
    open: &mut dyn FnMut() -> Result<File, String>,
    cancelled: Arc<AtomicBool>,
    progress: F,
) -> Result<(), String>
where
    F: FnMut(core::ExportProgress),
{
    let stream_cancelled = cancelled.clone();
    core::export_project_to(
        project,
        settings,
        Backend::new(maximum_temporal_decoders),
        Some(&mut || open_document_stream(open()?, stream_cancelled.clone())),
        cancelled,
        progress,
    )
}

pub fn open_document_stream(file: File, cancelled: Arc<AtomicBool>) -> Result<StreamIo, String> {
    let (writer, seekable) =
        document_writer(file, cancelled).map_err(|error| format!("Open export stream: {error}"))?;
    if seekable {
        StreamIo::from_write_seek(writer)
    } else {
        StreamIo::from_write(writer)
    }
    .map_err(|error| format!("Open export stream: {error}"))
}

pub fn write_document(file: File, bytes: &[u8], cancelled: Arc<AtomicBool>) -> io::Result<()> {
    document_writer(file, cancelled)?.0.write_all(bytes)
}

fn document_writer(
    mut file: File,
    cancelled: Arc<AtomicBool>,
) -> io::Result<(DocumentWriter, bool)> {
    let seekable = match file.stream_position() {
        Ok(_) => true,
        Err(error) if error.raw_os_error() == Some(libc::ESPIPE) => {
            let fd = file.as_raw_fd();
            let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            {
                return Err(io::Error::last_os_error());
            }
            false
        }
        Err(error) => return Err(error),
    };
    Ok((DocumentWriter { file, cancelled }, seekable))
}

struct DocumentWriter {
    file: File,
    cancelled: Arc<AtomicBool>,
}

impl Write for DocumentWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        const PROVIDER_POLL_MS: libc::c_int = 100;
        loop {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(io::Error::from_raw_os_error(libc::ECANCELED));
            }
            match self.file.write(data) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let mut descriptor = libc::pollfd {
                        fd: self.file.as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    if unsafe { libc::poll(&mut descriptor, 1, PROVIDER_POLL_MS) } < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for DocumentWriter {
    fn seek(&mut self, position: io::SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
