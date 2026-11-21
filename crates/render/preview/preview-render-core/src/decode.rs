use ffmpeg_next::{self as ffmpeg, util::format::pixel::Pixel};
use shrimply_math_core::Time;
use shrimply_preview_provider_skia::accuracy::CompositeAccuracy;
use shrimply_project_document::project::Asset;
#[cfg(windows)]
use shrimply_video_decoder::RgbFrameConverter;
use shrimply_video_decoder::{
    DecodeControl, DecodeMode, DecodeOutcome, DecodedVisual, DecoderBackend, VideoDecoderSession,
};
use shrimply_visual_frame::VideoFrame;
use skia_safe::Image;
#[cfg(windows)]
use skia_safe::{AlphaType, ColorType, Data, ImageInfo};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicU64},
};

#[derive(Clone)]
pub enum DecodedImage {
    Raster(Image),
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
    Video(Arc<VideoFrame>),
}

impl DecodedImage {
    pub fn width(&self) -> i32 {
        match self {
            Self::Raster(image) => image.width(),
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
            Self::Video(video) => video.native().width() as i32,
        }
    }
    pub fn height(&self) -> i32 {
        match self {
            Self::Raster(image) => image.height(),
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
            Self::Video(video) => video.native().height() as i32,
        }
    }
    pub fn source(&self) -> crate::Source {
        match self {
            Self::Raster(image) => crate::Source::Image(image.clone()),
            #[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
            Self::Video(video) => crate::Source::Video(video.clone()),
        }
    }
}

const RGBA_CHANNELS: usize = 4;
const SCRUB_CACHE_BYTES_PER_DECODER: u64 = 32 * 1024 * 1024;
// Retained hardware surfaces also consume slots in the codec's native frame pool.
const SCRUB_CACHE_FRAME_LIMIT: usize = 4;

struct CachedFrame {
    start: Time,
    end: Time,
    image: DecodedImage,
    bytes: u64,
}

pub struct Decoder {
    session: VideoDecoderSession,
    #[cfg(windows)]
    scaler: RgbFrameConverter,
    current: Option<(DecodedVisual, DecodedImage)>,
    cancellation: DecodeControl,
    history: VecDeque<CachedFrame>,
    history_bytes: u64,
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
            #[cfg(windows)]
            scaler: RgbFrameConverter::new(Pixel::RGBA)?,
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
        #[cfg(target_os = "android")]
        if frame.1.native().format() == Pixel::MEDIACODEC {
            let _measurement = shrimply_profiling::measure("Video decode / Prepare hardware image");
            frame
                .1
                .prepare_android(|| self.cancellation.superseded() || control.superseded())?;
        }
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
            let bytes = u64::try_from(image.width()).expect("positive image width")
                * u64::try_from(image.height()).expect("positive image height")
                * RGBA_CHANNELS as u64;
            #[cfg(target_os = "android")]
            let bytes = match &image {
                DecodedImage::Video(frame) => frame.bytes(),
                DecodedImage::Raster(_) => bytes,
            };
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

    #[cfg(not(windows))]
    fn image_for_frame(&self, source: &Arc<VideoFrame>) -> Result<DecodedImage, String> {
        if !matches!(
            source.native().format(),
            Pixel::VAAPI | Pixel::VIDEOTOOLBOX | Pixel::MEDIACODEC
        ) {
            return Err(format!(
                "Expected a hardware compositor frame, received {:?}",
                source.native().format()
            ));
        }
        Ok(DecodedImage::Video(source.clone()))
    }

    #[cfg(windows)]
    fn image_for_frame(&mut self, source: &Arc<VideoFrame>) -> Result<DecodedImage, String> {
        let decoded = source
            .download()?
            .ok_or("Expected a hardware frame for readback")?;
        let rgba = self.scaler.convert(&decoded)?;
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

fn select_backend(_parameters: &ffmpeg::codec::Parameters) -> Result<DecoderBackend, String> {
    #[cfg(target_os = "android")]
    {
        // The native image pool must fit retained scrub frames and the session's
        // current/lookahead/decode frames. All seek and cache policy stays here.
        const DECODER_WORKING_FRAMES: usize = 4;
        let native = unsafe { &*_parameters.as_ptr() };
        let width = u32::try_from(native.width).map_err(|_| "Invalid decoder width")?;
        let height = u32::try_from(native.height).map_err(|_| "Invalid decoder height")?;
        shrimply_render_vulkan::decoder_backend(
            width,
            height,
            (SCRUB_CACHE_FRAME_LIMIT + DECODER_WORKING_FRAMES) as u32,
        )
    }
    #[cfg(target_os = "linux")]
    {
        shrimply_render_vulkan::decoder_backend()
    }
    #[cfg(target_os = "macos")]
    {
        DecoderBackend::videotoolbox()
    }
    #[cfg(windows)]
    {
        DecoderBackend::hardware_for_readback(_parameters)
    }
}
