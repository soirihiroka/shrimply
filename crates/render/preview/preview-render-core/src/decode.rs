use ffmpeg_next::{self as ffmpeg, frame, software, util::format::pixel::Pixel};
use shrimply_math_core::Time;
use shrimply_preview_provider_skia::accuracy::CompositeAccuracy;
use shrimply_project_document::project::Asset;
use shrimply_video_decoder::{
    DecodeControl, DecodeMode, DecodeOutcome, DecodedVisual, DecoderBackend, VideoDecoderSession,
};
use shrimply_visual_frame::VideoFrame;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicU64},
};

#[derive(Clone)]
pub enum DecodedImage {
    Raster(Image),
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    Video(Arc<VideoFrame>),
}

impl DecodedImage {
    pub fn width(&self) -> i32 {
        match self {
            Self::Raster(image) => image.width(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Self::Video(video) => video.native().width() as i32,
        }
    }
    pub fn height(&self) -> i32 {
        match self {
            Self::Raster(image) => image.height(),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Self::Video(video) => video.native().height() as i32,
        }
    }
    pub fn source(&self) -> crate::Source {
        match self {
            Self::Raster(image) => crate::Source::Image(image.clone()),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            Self::Video(video) => crate::Source::Video(video.clone()),
        }
    }
}

const RGBA_CHANNELS: usize = 4;
const SCRUB_CACHE_BYTES_PER_DECODER: usize = 32 * 1024 * 1024;
// Retained hardware surfaces also consume slots in the codec's native frame pool.
const SCRUB_CACHE_FRAME_LIMIT: usize = 4;

struct CachedFrame {
    start: Time,
    end: Time,
    image: DecodedImage,
    bytes: usize,
}

pub struct Decoder {
    session: VideoDecoderSession,
    scaler: Option<software::scaling::Context>,
    current: Option<(DecodedVisual, DecodedImage)>,
    cancellation: DecodeControl,
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
        let cancellation = DecodeControl::new(epoch, cancellation);
        let session = VideoDecoderSession::open(
            &file.snapshot()?,
            track,
            select_backend,
            Some(cancellation.clone()),
        )?;
        Ok(Self {
            session,
            scaler: None,
            current: None,
            cancellation,
            history: VecDeque::new(),
            history_bytes: 0,
        })
    }

    pub fn image(
        &mut self,
        time: Time,
        accuracy: CompositeAccuracy,
        latest: &Arc<AtomicU64>,
        generation: u64,
    ) -> Result<Option<(Time, DecodedImage)>, String> {
        let control = DecodeControl::new(generation, latest.clone());
        if self.cancellation.superseded() || control.superseded() {
            return Err("preview request superseded".into());
        }
        if let Some(cached) = self
            .history
            .iter()
            .find(|frame| frame.start <= time && time < frame.end)
        {
            return Ok(Some((cached.start, cached.image.clone())));
        }
        let mode = if accuracy.continuous_playback() {
            DecodeMode::Continuous
        } else if accuracy.local_scrub() {
            DecodeMode::LocalScrub
        } else if accuracy.time_accurate() {
            DecodeMode::Accurate
        } else {
            DecodeMode::BestEffort
        };
        let outcome = self.session.frame(
            time,
            self.current.as_ref().map(|(frame, _)| frame.clone()),
            [Some(&self.cancellation), Some(&control)],
            mode,
        )?;
        let frame = match outcome {
            DecodeOutcome::Frame(Some(frame)) => frame,
            DecodeOutcome::Frame(None) => return Ok(None),
            DecodeOutcome::Superseded(_) => return Err("preview request superseded".into()),
        };
        let image = if let Some(((_, previous), image)) = &self.current
            && Arc::ptr_eq(previous, &frame.1)
        {
            image.clone()
        } else {
            self.image_for_frame(&frame.1)?
        };
        if let Some(end) = self.session.next_frame_time().filter(|end| *end > frame.0)
            && !self.history.iter().any(|cached| cached.start == frame.0)
        {
            let bytes = image.width() as usize * image.height() as usize * RGBA_CHANNELS;
            if bytes <= SCRUB_CACHE_BYTES_PER_DECODER {
                while self.history_bytes + bytes > SCRUB_CACHE_BYTES_PER_DECODER
                    || self.history.len() >= SCRUB_CACHE_FRAME_LIMIT
                {
                    let old = self
                        .history
                        .pop_front()
                        .expect("scrub cache accounts for retained frames");
                    self.history_bytes -= old.bytes;
                }
                self.history.push_back(CachedFrame {
                    start: frame.0,
                    end,
                    image: image.clone(),
                    bytes,
                });
                self.history_bytes += bytes;
            }
        }
        let position = frame.0;
        self.current = Some((frame, image.clone()));
        Ok(Some((position, image)))
    }

    fn image_for_frame(&mut self, source: &Arc<VideoFrame>) -> Result<DecodedImage, String> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if matches!(source.native().format(), Pixel::VAAPI | Pixel::VIDEOTOOLBOX) {
            return Ok(DecodedImage::Video(source.clone()));
        }
        // Raster consumers need a software image; hardware compositors never use this path.
        let decoded = source.native();
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
            .run(decoded, &mut rgba)
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
        Ok(DecodedImage::Raster(image))
    }
}

fn select_backend(codec: ffmpeg::codec::Id) -> Result<DecoderBackend, String> {
    #[cfg(target_os = "linux")]
    if matches!(
        codec,
        ffmpeg::codec::Id::H264
            | ffmpeg::codec::Id::HEVC
            | ffmpeg::codec::Id::VP8
            | ffmpeg::codec::Id::VP9
            | ffmpeg::codec::Id::MPEG2VIDEO
            | ffmpeg::codec::Id::MJPEG
    ) {
        return shrimply_render_vulkan::decoder_backend();
    }
    #[cfg(target_os = "macos")]
    match DecoderBackend::videotoolbox() {
        Ok(DecoderBackend::Hardware(device)) => return Ok(DecoderBackend::PreferHardware(device)),
        Ok(_) => unreachable!("VideoToolbox must create a hardware device"),
        Err(error) => {
            tracing::info!(?codec, %error, "VideoToolbox unavailable; using software decoding")
        }
    }
    let _ = codec;
    Ok(DecoderBackend::Software)
}
