use ffmpeg_next::{
    self as ffmpeg, codec, format, frame, media, software, util::format::pixel::Pixel,
};
use shrimply_math_core::Time;
use shrimply_preview_core::accuracy::CompositeAccuracy;
use shrimply_project::project::Asset;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

const RGBA_CHANNELS: usize = 4;
pub const SEQUENTIAL_DECODE_LIMIT_SECONDS: i64 = 1;
const SCRUB_CACHE_BYTES_PER_DECODER: usize = 32 * 1024 * 1024;

struct CachedFrame {
    start: Time,
    end: Time,
    image: Image,
}

pub struct Decoder {
    input: format::context::Input,
    decoder: codec::decoder::Video,
    scaler: Option<software::scaling::Context>,
    #[cfg(target_os = "macos")]
    output_format: Option<Pixel>,
    stream: usize,
    time_base: ffmpeg::Rational,
    origin: i64,
    current: Option<(Time, Image)>,
    next: Option<(Time, Image)>,
    eof: bool,
    cancellation: Arc<AtomicU64>,
    epoch: u64,
    seek_target: Option<Time>,
    history: VecDeque<CachedFrame>,
    history_bytes: usize,
}

impl Decoder {
    pub fn new(
        file: &Asset,
        track: u32,
        cancellation: Arc<AtomicU64>,
        epoch: u64,
    ) -> Result<Self, String> {
        ffmpeg::init().map_err(|e| e.to_string())?;
        let interrupt = cancellation.clone();
        let input = format::input_with_interrupt(&file.path(), move || {
            interrupt.load(Ordering::Relaxed) != epoch
        })
        .map_err(|e| e.to_string())?;
        let stream = input
            .streams()
            .filter(|s| s.parameters().medium() == media::Type::Video)
            .nth(track as usize)
            .ok_or("video stream does not exist")?;
        let decoder = open_decoder(stream.parameters(), stream.time_base())?;
        let index = stream.index();
        let time_base = stream.time_base();
        let origin = if stream.start_time() == ffmpeg::ffi::AV_NOPTS_VALUE {
            0
        } else {
            stream.start_time()
        };
        Ok(Self {
            input,
            decoder,
            scaler: None,
            #[cfg(target_os = "macos")]
            output_format: None,
            stream: index,
            time_base,
            origin,
            current: None,
            next: None,
            eof: false,
            cancellation,
            epoch,
            seek_target: None,
            history: VecDeque::new(),
            history_bytes: 0,
        })
    }

    pub fn image(
        &mut self,
        time: Time,
        accuracy: CompositeAccuracy,
        latest: &AtomicU64,
        generation: u64,
    ) -> Result<Image, String> {
        if let Some(cached) = self
            .history
            .iter()
            .find(|frame| frame.start <= time && time < frame.end)
        {
            return Ok(cached.image.clone());
        }
        let distant = self.current.as_ref().is_some_and(|(current, _)| {
            time < *current
                || time
                    > current.saturating_add(Time::from_seconds(SEQUENTIAL_DECODE_LIMIT_SECONDS))
        });
        let continuing_seek = self.seek_target == Some(time)
            && self
                .current
                .as_ref()
                .is_none_or(|(current, _)| *current <= time);
        let seek = (distant || self.current.is_none() && time > Time::ZERO) && !continuing_seek;
        if seek {
            let absolute =
                time.saturating_add(crate::math::frame_time(self.origin, self.time_base));
            let timestamp =
                crate::math::timestamp(absolute, ffmpeg::Rational(1, ffmpeg::ffi::AV_TIME_BASE));
            self.input
                .seek(timestamp, ..timestamp)
                .map_err(|e| e.to_string())?;
            self.decoder.flush();
            self.current = None;
            self.next = None;
            self.eof = false;
            self.seek_target = Some(time);
        }
        if !accuracy.time_accurate() && (seek || self.current.is_none()) {
            // Match GTK's random interactive seek: display the first decoded
            // frame, then decode precisely when the settled request arrives.
            let frame = self
                .read_frame(latest, generation)?
                .ok_or("video contains no decodable frame")?;
            let image = frame.1.clone();
            self.current = Some(frame);
            return Ok(image);
        }
        loop {
            if self.cancellation.load(Ordering::Relaxed) != self.epoch
                || latest.load(Ordering::Relaxed) != generation
            {
                return Err("preview request cancelled".into());
            }
            if let Some((next_time, _)) = &self.next {
                if *next_time > time && self.current.is_some() {
                    break;
                }
                self.current = self.next.take();
            }
            match self.read_frame(latest, generation)? {
                Some(frame) => {
                    self.remember_current(frame.0);
                    self.next = Some(frame);
                }
                None => break,
            }
        }
        self.current
            .as_ref()
            .map(|(_, image)| image.clone())
            .ok_or_else(|| "video contains no decodable frame".into())
    }

    fn remember_current(&mut self, end: Time) {
        let Some((start, image)) = &self.current else {
            return;
        };
        if *start >= end || self.history.iter().any(|frame| frame.start == *start) {
            return;
        }
        let bytes = image.width() as usize * image.height() as usize * RGBA_CHANNELS;
        if bytes > SCRUB_CACHE_BYTES_PER_DECODER {
            return;
        }
        while self.history_bytes + bytes > SCRUB_CACHE_BYTES_PER_DECODER {
            let old = self
                .history
                .pop_front()
                .expect("scrub cache accounts for retained frames");
            self.history_bytes -=
                old.image.width() as usize * old.image.height() as usize * RGBA_CHANNELS;
        }
        self.history.push_back(CachedFrame {
            start: *start,
            end,
            image: image.clone(),
        });
        self.history_bytes += bytes;
    }

    fn read_frame(
        &mut self,
        latest: &AtomicU64,
        generation: u64,
    ) -> Result<Option<(Time, Image)>, String> {
        let mut decoded = frame::Video::empty();
        loop {
            if latest.load(Ordering::Relaxed) != generation {
                return Err("preview request superseded".into());
            }
            match self.decoder.receive_frame(&mut decoded) {
                Ok(()) => break,
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::error::EAGAIN => {}
                Err(error) => return Err(error.to_string()),
            }
            if self.eof {
                return Ok(None);
            }
            let packet = loop {
                if latest.load(Ordering::Relaxed) != generation {
                    return Err("preview request superseded".into());
                }
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) if packet.stream() == self.stream => break Some(packet),
                    Ok(()) => {}
                    Err(ffmpeg::Error::Eof) => break None,
                    Err(error) => return Err(error.to_string()),
                }
            };
            if let Some(packet) = packet {
                self.decoder
                    .send_packet(&packet)
                    .map_err(|e| e.to_string())?;
            } else {
                self.decoder.send_eof().map_err(|e| e.to_string())?;
                self.eof = true;
            }
        }
        let time = crate::math::frame_time(
            decoded
                .timestamp()
                .ok_or("decoded frame has no timestamp")?
                - self.origin,
            self.time_base,
        );
        #[cfg(target_os = "macos")]
        {
            if self.output_format != Some(decoded.format()) {
                self.output_format = Some(decoded.format());
                if decoded.format() == Pixel::VIDEOTOOLBOX {
                    tracing::info!("Preview is decoding VideoToolbox frames");
                } else {
                    tracing::info!(format = ?decoded.format(), "Preview is decoding software frames");
                }
            }
            if decoded.format() == Pixel::VIDEOTOOLBOX {
                let mut downloaded = frame::Video::empty();
                // FFmpeg owns the CVPixelBuffer in decoded. Transfer to a new
                // owned software AVFrame before passing pixels to swscale/Skia.
                let result = unsafe {
                    ffmpeg::ffi::av_hwframe_transfer_data(
                        downloaded.as_mut_ptr(),
                        decoded.as_ptr(),
                        0,
                    )
                };
                if result < 0 {
                    return Err(format!(
                        "could not transfer VideoToolbox frame: {}",
                        ffmpeg::Error::from(result)
                    ));
                }
                let result = unsafe {
                    ffmpeg::ffi::av_frame_copy_props(downloaded.as_mut_ptr(), decoded.as_ptr())
                };
                if result < 0 {
                    return Err(format!(
                        "could not copy VideoToolbox frame properties: {}",
                        ffmpeg::Error::from(result)
                    ));
                }
                decoded = downloaded;
            }
        }
        let definition = software::scaling::context::Definition {
            format: decoded.format(),
            width: decoded.width(),
            height: decoded.height(),
        };
        if self
            .scaler
            .as_ref()
            .is_none_or(|scaler| *scaler.input() != definition)
        {
            self.scaler = Some(
                software::scaling::Context::get(
                    definition.format,
                    definition.width,
                    definition.height,
                    Pixel::RGBA,
                    definition.width,
                    definition.height,
                    software::scaling::Flags::BILINEAR,
                )
                .map_err(|e| e.to_string())?,
            );
        }
        let mut rgba = frame::Video::empty();
        self.scaler
            .as_mut()
            .expect("video scaler initialized")
            .run(&decoded, &mut rgba)
            .map_err(|e| e.to_string())?;
        let row_bytes = rgba.width() as usize * RGBA_CHANNELS;
        let mut pixels = Vec::with_capacity(row_bytes * rgba.height() as usize);
        for row in rgba
            .data(0)
            .chunks(rgba.stride(0))
            .take(rgba.height() as usize)
        {
            pixels.extend_from_slice(&row[..row_bytes]);
        }
        let info = ImageInfo::new(
            (rgba.width() as i32, rgba.height() as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let image = skia_safe::images::raster_from_data(&info, Data::new_copy(&pixels), row_bytes)
            .ok_or("create decoded Skia image")?;
        Ok(Some((time, image)))
    }
}

fn open_decoder(
    parameters: codec::Parameters,
    time_base: ffmpeg::Rational,
) -> Result<codec::decoder::Video, String> {
    #[cfg(target_os = "macos")]
    match videotoolbox::open(&parameters, time_base) {
        Ok(decoder) => return Ok(decoder),
        Err(error) => {
            tracing::info!(codec = ?parameters.id(), %error, "VideoToolbox is unavailable; using software decoding")
        }
    }
    let mut context = codec::context::Context::from_parameters(parameters)
        .map_err(|error| error.to_string())?
        .decoder();
    context.set_packet_time_base(time_base);
    context.video().map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
mod videotoolbox {
    use super::*;
    use ffmpeg::ffi as sys;
    use std::ptr;

    pub(super) fn open(
        parameters: &codec::Parameters,
        time_base: ffmpeg::Rational,
    ) -> Result<codec::decoder::Video, String> {
        // The default decoder can be software-only (for example libdav1d),
        // while another decoder for the same codec exposes VideoToolbox.
        let mut iterator = ptr::null_mut();
        let codec = 'codecs: loop {
            let codec = unsafe { sys::av_codec_iterate(&mut iterator) };
            if codec.is_null() {
                return Err("codec does not expose a VideoToolbox hardware configuration".into());
            }
            if unsafe { sys::av_codec_is_decoder(codec) } == 0
                || unsafe { (*codec).id } != parameters.id().into()
            {
                continue;
            }
            let mut index = 0;
            loop {
                // FFmpeg returns immutable codec-owned configuration descriptors.
                let config = unsafe { sys::avcodec_get_hw_config(codec, index) };
                if config.is_null() {
                    break;
                }
                let config = unsafe { &*config };
                if config.device_type == sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX
                    && config.pix_fmt == sys::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX
                    && config.methods & sys::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
                {
                    break 'codecs unsafe { ffmpeg::Codec::wrap(codec) };
                }
                index += 1;
            }
        };
        let mut context = codec::context::Context::from_parameters(parameters.clone())
            .map_err(|error| error.to_string())?
            .decoder();
        context.set_packet_time_base(time_base);
        let mut device = ptr::null_mut();
        let result = unsafe {
            sys::av_hwdevice_ctx_create(
                &mut device,
                sys::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        if result < 0 {
            unsafe {
                sys::av_buffer_unref(&mut device);
            }
            return Err(format!(
                "could not create VideoToolbox device: {}",
                ffmpeg::Error::from(result)
            ));
        }
        // Ownership of the device reference passes to AVCodecContext, which
        // releases it even if opening the decoder fails.
        unsafe {
            (*context.as_mut_ptr()).hw_device_ctx = device;
            (*context.as_mut_ptr()).get_format = Some(get_format);
        }
        context
            .open_as(codec)
            .and_then(|opened| opened.video())
            .map_err(|error| error.to_string())
    }

    unsafe extern "C" fn get_format(
        _context: *mut sys::AVCodecContext,
        formats: *const sys::AVPixelFormat,
    ) -> sys::AVPixelFormat {
        let mut cursor = formats;
        let mut software = sys::AVPixelFormat::AV_PIX_FMT_NONE;
        // The callback receives an AV_PIX_FMT_NONE-terminated array. Never
        // return a format absent from that array; FFmpeg may call again with
        // VideoToolbox removed after hardware initialization fails.
        while !cursor.is_null() {
            let format = unsafe { *cursor };
            if format == sys::AVPixelFormat::AV_PIX_FMT_NONE {
                break;
            }
            if format == sys::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
                return format;
            }
            let descriptor = unsafe { sys::av_pix_fmt_desc_get(format) };
            if software == sys::AVPixelFormat::AV_PIX_FMT_NONE
                && !descriptor.is_null()
                && unsafe { (*descriptor).flags } & (sys::AV_PIX_FMT_FLAG_HWACCEL as u64) == 0
            {
                software = format;
            }
            cursor = unsafe { cursor.add(1) };
        }
        software
    }
}
