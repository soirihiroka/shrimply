use std::io::Cursor;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::svg_render::{PreparedSvg, SvgVectorVisualParams, svg_vector_visual};
use crate::visual_source::VisualSourceCache;
use crate::visual_source::{
    VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest,
};
use ffmpeg::{format, media};
use ffmpeg_next as ffmpeg;
use rayon::prelude::*;
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_gpu_memory::{ResourceKey, global as gpu_memory};
use shrimply_project::project::{Time, VideoItem, VideoItemContent};
use shrimply_video_modifiers::ModifierEffect;
use shrimply_video_modifiers::vectorize::{
    MAX_ANGLE_DEGREES, MAX_BINARY_THRESHOLD, MAX_COLOR_PRECISION, MAX_GRADIENT_STEP,
    MAX_ITERATIONS, MAX_PATH_PRECISION, MAX_SEGMENT_LENGTH, MAX_SPECKLE_SIZE, MIN_COLOR_PRECISION,
    MIN_SEGMENT_LENGTH, VectorizeColorMode, VectorizeHierarchy, VectorizeModifier,
    VectorizePathMode,
};
use uuid::Uuid;

const PARALLEL_RGBA_COPY_MIN_BYTES: usize = 1024 * 1024;
const VECTORIZE_DEBOUNCE: Duration = Duration::from_millis(200);

pub struct ImageDecodeSession {
    file: Asset,
    snapshot: AssetSnapshot,
    width: u32,
    height: u32,
    kind: ImageDecodeKind,
    raster: Option<RasterDecoder>,
    gif_pending: Option<Receiver<Result<DecodedGif, String>>>,
    gif_frames: Vec<GifFrame>,
    gif_error: Option<String>,
    vectorize_request: Option<VectorizeRequest>,
    vectorize_pending: Option<VectorizePending>,
    vectorized: Option<VectorizedImage>,
    vectorize_error: Option<(Vec<u8>, String)>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ImageDecodeKind {
    Image,
    Gif,
}

struct VectorizeRequest {
    key: Vec<u8>,
    modifier: VectorizeModifier,
    changed_at: Instant,
}

struct VectorizePending {
    key: Vec<u8>,
    result: Receiver<Result<VectorizedOutput, String>>,
}

struct VectorizedImage {
    key: Vec<u8>,
    source_key: ResourceKey,
    width: u32,
    height: u32,
}

struct VectorizedOutput {
    key: Vec<u8>,
    svg: String,
    width: u32,
    height: u32,
}

struct DecodedRgba {
    pixels: Vec<u8>,
    width: u32,
    height: u32,
}

struct DecodedGif {
    frames: Vec<(Time, VisualFrame)>,
    width: u32,
    height: u32,
}

struct GifFrame {
    position: Time,
    source: VisualFrame,
    gpu: Option<Rc<VisualFrame>>,
}

struct RasterDecoder {
    input: format::context::Input,
    stream_index: usize,
    decoder: ffmpeg::decoder::Video,
    scaler: ffmpeg::software::scaling::context::Context,
    eof: bool,
}

impl ImageDecodeSession {
    fn image_key(&self) -> ResourceKey {
        let mut discriminator = Vec::new();
        discriminator.extend_from_slice(b"rgba");
        discriminator.push(0);
        discriminator.extend_from_slice(self.snapshot.cache_key().as_bytes());
        ResourceKey::new(self.snapshot.path().to_path_buf(), discriminator)
    }

    fn gpu_image_key(&self) -> ResourceKey {
        let mut discriminator = Vec::new();
        discriminator.extend_from_slice(b"rgba-gpu\0");
        discriminator.extend_from_slice(self.snapshot.cache_key().as_bytes());
        ResourceKey::new(self.snapshot.path().to_path_buf(), discriminator)
    }

    fn upload_still_frame(
        &self,
        source: &VisualFrame,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Rc<VisualFrame>, String> {
        let mut frame = compositor.upload_frame(source)?;
        if let Some(retained) =
            compositor.retain_host_backed_frame(&frame, "still image preview cache")?
        {
            gpu_memory().insert_resource(self.gpu_image_key(), 0, retained.clone())?;
            frame = retained;
        }
        compositor.prepare_host_backed_frame(&frame, "still image preview")?;
        Ok(Rc::new(frame))
    }

    fn vectorized_key(&self, key: &[u8]) -> ResourceKey {
        let mut discriminator = b"vectorized-svg\0".to_vec();
        discriminator.extend_from_slice(key);
        ResourceKey::new(self.snapshot.path().to_path_buf(), discriminator)
    }

    pub fn new(item: &VideoItem) -> Result<Self, String> {
        let kind = match &item.content {
            VideoItemContent::Image => ImageDecodeKind::Image,
            VideoItemContent::Gif => ImageDecodeKind::Gif,
            _ => return Err("visual media decoder received a non-image item".to_string()),
        };
        Ok(Self {
            file: item.file.clone(),
            snapshot: item.file.snapshot()?,
            width: item.source_width.max(1),
            height: item.source_height.max(1),
            kind,
            raster: None,
            gif_pending: None,
            gif_frames: Vec::new(),
            gif_error: None,
            vectorize_request: None,
            vectorize_pending: None,
            vectorized: None,
            vectorize_error: None,
        })
    }

    fn request_vectorization(&mut self, item: &VideoItem) -> Result<Option<Vec<u8>>, String> {
        let Some(modifier) = item
            .modifiers
            .iter()
            .find(|modifier| modifier.enabled)
            .and_then(|modifier| match &modifier.effect {
                ModifierEffect::Vectorize(modifier) => Some(modifier),
                _ => None,
            })
        else {
            self.vectorize_request = None;
            return Ok(None);
        };
        let mut key = serde_json::to_vec(modifier)
            .map_err(|error| format!("serialize Vectorize request: {error}"))?;
        key.extend_from_slice(self.snapshot.cache_key().as_bytes());

        if self
            .vectorize_request
            .as_ref()
            .is_none_or(|request| request.key != key)
        {
            self.vectorize_request = Some(VectorizeRequest {
                key: key.clone(),
                modifier: modifier.clone(),
                changed_at: Instant::now(),
            });
        }
        self.poll_vectorization();
        if self.vectorized.as_ref().is_some_and(|image| {
            image.key == key && !gpu_memory().contains_resource(&image.source_key)
        }) {
            self.vectorized = None;
        }
        let already_resolved = self
            .vectorized
            .as_ref()
            .is_some_and(|image| image.key == key)
            || self
                .vectorize_error
                .as_ref()
                .is_some_and(|(error_key, _)| *error_key == key);
        if self.vectorize_pending.is_none()
            && !already_resolved
            && self
                .vectorize_request
                .as_ref()
                .is_some_and(|request| request.changed_at.elapsed() >= VECTORIZE_DEBOUNCE)
        {
            let request = self
                .vectorize_request
                .as_ref()
                .expect("vectorization request disappeared");
            let file = self.file.clone();
            let trace_key = request.key.clone();
            let modifier = request.modifier.clone();
            let (sender, result) = sync_channel(1);
            self.vectorize_pending = Some(VectorizePending {
                key: trace_key.clone(),
                result,
            });
            rayon::spawn(move || {
                let traced = vectorize_image(&file, &modifier, trace_key);
                let _ = sender.send(traced);
            });
            shrimply_benchmarking::increment("Image vectorization / Prepared requests submitted");
        }
        Ok(Some(key))
    }

    fn poll_vectorization(&mut self) {
        let Some(pending) = &self.vectorize_pending else {
            return;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Vectorize background worker stopped unexpectedly".to_string())
            }
        };
        let pending = self
            .vectorize_pending
            .take()
            .expect("polled Vectorize request disappeared");
        if self
            .vectorize_request
            .as_ref()
            .is_none_or(|request| request.key != pending.key)
        {
            return;
        }
        match result {
            Ok(output) => {
                let source_key = self.vectorized_key(&output.key);
                let source_bytes = u64::try_from(output.svg.len())
                    .map_err(|_| "vectorized SVG source size exceeds u64".to_string());
                let prepared = source_bytes.and_then(|bytes| {
                    PreparedSvg::new(output.svg).map(|prepared| (bytes, prepared))
                });
                let (source_bytes, prepared) = match prepared {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        self.vectorize_error = Some((pending.key, error));
                        return;
                    }
                };
                if let Err(error) =
                    gpu_memory().insert_resource(source_key.clone(), source_bytes, prepared)
                {
                    self.vectorize_error = Some((pending.key, error));
                    return;
                }
                self.width = output.width;
                self.height = output.height;
                self.vectorized = Some(VectorizedImage {
                    key: output.key,
                    source_key,
                    width: output.width,
                    height: output.height,
                });
                self.vectorize_error = None;
            }
            Err(error) => self.vectorize_error = Some((pending.key, error)),
        }
    }

    fn vectorized_render(
        &self,
        key: &[u8],
        request: VisualRenderRequest<'_>,
    ) -> Result<VisualRender, String> {
        if let Some((_, error)) = self
            .vectorize_error
            .as_ref()
            .filter(|(error_key, _)| error_key == key)
        {
            return Err(error.clone());
        }
        let Some(image) = &self.vectorized else {
            return Ok(VisualRender::Loading(
                shrimply_project::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            ));
        };
        let Some(prepared_svg) = gpu_memory().get_resource(&image.source_key)? else {
            return Ok(VisualRender::Loading(
                shrimply_project::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            ));
        };
        let visual = svg_vector_visual(
            SvgVectorVisualParams {
                prepared_svg,
                root_size: shrimply_project::project::CanvasSize {
                    width: image.width,
                    height: image.height,
                },
                surface_size: request.render_canvas,
                canvas_size: request.project.canvas_size,
                evaluation: shrimply_evaluation::VisualEvaluation::for_item_with_audio(
                    request.project,
                    request.item,
                    request.position,
                    request.audio_analysis,
                ),
                transition: request.generated_transition,
            },
            request.state,
        )?;
        Ok(if image.key == key {
            VisualRender::Ready(visual)
        } else {
            VisualRender::LoadingPlaceholder(visual)
        })
    }

    pub fn frame_at(
        &mut self,
        source_position: Time,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Option<Rc<VisualFrame>>, String> {
        match self.kind {
            ImageDecodeKind::Image => self.raster_frame_at(compositor),
            ImageDecodeKind::Gif => self.gif_frame_at(source_position, compositor),
        }
    }

    fn request_gif(&mut self) {
        if self.gif_pending.is_some() || !self.gif_frames.is_empty() || self.gif_error.is_some() {
            return;
        }
        let snapshot = self.snapshot.clone();
        let (sender, result) = sync_channel(1);
        self.gif_pending = Some(result);
        rayon::spawn(move || {
            let _ = sender.send(decode_gif(snapshot));
        });
        shrimply_benchmarking::increment("GIF decode / Prepared requests submitted");
    }

    fn poll_gif(&mut self) {
        let Some(pending) = &self.gif_pending else {
            return;
        };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("GIF decoder stopped unexpectedly".to_string()),
        };
        self.gif_pending = None;
        match result {
            Ok(decoded) => {
                self.width = decoded.width;
                self.height = decoded.height;
                self.gif_frames = decoded
                    .frames
                    .into_iter()
                    .map(|(position, source)| GifFrame {
                        position,
                        source,
                        gpu: None,
                    })
                    .collect();
            }
            Err(error) => self.gif_error = Some(error),
        }
    }

    fn gif_frame_at(
        &mut self,
        target: Time,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Option<Rc<VisualFrame>>, String> {
        self.poll_gif();
        if let Some(error) = &self.gif_error {
            return Err(error.clone());
        }
        if self.gif_frames.is_empty() {
            self.request_gif();
            return Ok(None);
        }
        let Some(index) = self
            .gif_frames
            .partition_point(|frame| frame.position <= target)
            .checked_sub(1)
        else {
            return Ok(None);
        };
        let frame = &mut self.gif_frames[index];
        if frame.gpu.is_none() {
            shrimply_benchmarking::increment("GIF frame GPU residency / Miss");
            frame.gpu = Some(Rc::new(compositor.upload_frame(&frame.source)?));
        } else {
            shrimply_benchmarking::increment("GIF frame GPU residency / Hit");
        }
        let frame = frame.gpu.as_ref().expect("GIF frame was just uploaded");
        compositor.prepare_host_backed_frame(frame, "resident GIF frame")?;
        Ok(Some(frame.clone()))
    }

    fn raster_frame_at(
        &mut self,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Option<Rc<VisualFrame>>, String> {
        if let Some(frame) = gpu_memory().get_resource::<VisualFrame>(&self.gpu_image_key())? {
            shrimply_benchmarking::increment("Image GPU residency / Hit");
            compositor.prepare_host_backed_frame(&frame, "cached still image preview")?;
            return Ok(Some(Rc::new((*frame).clone())));
        }
        shrimply_benchmarking::increment("Image GPU residency / Miss");
        if let Some(frame) = gpu_memory().get_resource::<VisualFrame>(&self.image_key())? {
            return self.upload_still_frame(&frame, compositor).map(Some);
        }
        if gpu_memory().contains_resource(&self.image_key()) {
            return Ok(None);
        }
        if self.raster.is_none() {
            self.open_raster_decoder()?;
        }

        loop {
            if self.receive_frame()? {
                let frame = gpu_memory()
                    .get_resource::<VisualFrame>(&self.image_key())?
                    .ok_or_else(|| "decoded image source disappeared".to_string())?;
                return self.upload_still_frame(&frame, compositor).map(Some);
            }
            let Some(raster) = &mut self.raster else {
                return Ok(None);
            };
            if raster.eof {
                return Ok(None);
            }
            let mut sent_packet = false;
            for (stream, packet) in raster.input.packets() {
                if stream.index() != raster.stream_index {
                    continue;
                }
                raster
                    .decoder
                    .send_packet(&packet)
                    .map_err(|error| error.to_string())?;
                sent_packet = true;
                break;
            }
            if !sent_packet {
                match raster.decoder.send_eof() {
                    Ok(()) | Err(ffmpeg::Error::Eof) => raster.eof = true,
                    Err(error) => return Err(error.to_string()),
                }
            }
        }
    }

    fn open_raster_decoder(&mut self) -> Result<(), String> {
        ffmpeg::init().map_err(|error| error.to_string())?;
        let input = format::input(&self.file)
            .map_err(|error| format!("could not open {}: {error}", self.file.display()))?;
        let (stream_index, parameters) = {
            let stream = input
                .streams()
                .find(|stream| stream.parameters().medium() == media::Type::Video)
                .ok_or_else(|| format!("{} has no video stream", self.file.display()))?;
            (stream.index(), stream.parameters())
        };
        let context = ffmpeg::codec::context::Context::from_parameters(parameters)
            .map_err(|error| error.to_string())?;
        let decoder = context
            .decoder()
            .video()
            .map_err(|error| error.to_string())?;
        self.width = decoder.width().max(1);
        self.height = decoder.height().max(1);
        let scaler = ffmpeg::software::scaling::context::Context::get(
            decoder.format(),
            self.width,
            self.height,
            format::Pixel::RGBA,
            self.width,
            self.height,
            ffmpeg::software::scaling::flag::Flags::BILINEAR,
        )
        .map_err(|error| error.to_string())?;
        self.raster = Some(RasterDecoder {
            input,
            stream_index,
            decoder,
            scaler,
            eof: false,
        });
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<bool, String> {
        let Some(raster) = &mut self.raster else {
            return Ok(false);
        };
        let mut decoded = ffmpeg::frame::Video::empty();
        if raster.decoder.receive_frame(&mut decoded).is_err() {
            return Ok(false);
        }
        let mut rgba = ffmpeg::frame::Video::empty();
        raster
            .scaler
            .run(&decoded, &mut rgba)
            .map_err(|error| error.to_string())?;
        let pixels = tight_rgba_from_rows(rgba.data(0), rgba.stride(0), self.width, self.height)?;
        let source = VisualFrame::from_rgba_bytes(self.width, self.height, pixels)?;
        gpu_memory().insert_resource(self.image_key(), source.bytes(), source)?;
        Ok(true)
    }
}

impl VisualElement for ImageDecodeSession {
    fn matches(
        &self,
        item: &VideoItem,
        _canvas_size: shrimply_project::project::CanvasSize,
    ) -> bool {
        self.file == item.file
            && matches!(
                (self.kind, &item.content),
                (ImageDecodeKind::Image, VideoItemContent::Image)
                    | (ImageDecodeKind::Gif, VideoItemContent::Gif)
            )
            && self.snapshot.is_current()
    }

    fn prepare(
        &mut self,
        request: VisualPrepareRequest<'_>,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<(), String> {
        if self.kind == ImageDecodeKind::Gif {
            self.request_gif();
            return Ok(());
        }
        if self.request_vectorization(request.item)?.is_some() {
            return Ok(());
        }
        let file = self.file.clone();
        let key = self.image_key();
        let snapshot = self.snapshot.clone();
        if !gpu_memory().begin_resource_load(key.clone()) {
            return Ok(());
        }
        rayon::spawn(move || {
            let decoded = decode_still_image(&file, snapshot);
            let bytes = decoded.as_ref().map_or(0, VisualFrame::bytes);
            if let Err(error) = gpu_memory().finish_resource_load(key, bytes, decoded) {
                tracing::error!(file = %file.display(), %error, "could not finish loading image");
            }
        });
        shrimply_benchmarking::increment("Image decode / Prepared requests submitted");
        Ok(())
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        if self.kind == ImageDecodeKind::Image
            && let Some(key) = self.request_vectorization(request.item)?
        {
            return self.vectorized_render(&key, request);
        }
        let Some(source_position) =
            shrimply_project::project::video_source_time_at(request.item, request.position)
        else {
            return Ok(VisualRender::Empty);
        };
        match self.frame_at(source_position, compositor)? {
            Some(frame) => {
                self.width = frame.width();
                self.height = frame.height();
                Ok(VisualRender::Ready(crate::layer::Visual::Raster(
                    crate::layer::RasterVisual::materialized(
                        crate::layer::GpuFrame::Rgba(frame),
                        request.state,
                    ),
                )))
            }
            None => Ok(VisualRender::Loading(
                shrimply_project::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            )),
        }
    }
}

fn decode_gif(snapshot: AssetSnapshot) -> Result<DecodedGif, String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let source = snapshot.read()?;
    let io = format::context::StreamIo::from_read_seek(Cursor::new(source))
        .map_err(|error| error.to_string())?;
    let filename = snapshot.path().to_string_lossy();
    let mut input = format::input_from_stream(io, Some(&filename), None).map_err(|error| {
        format!(
            "could not open {} from memory: {error}",
            snapshot.path().display()
        )
    })?;
    let (stream_index, time_base, stream_start_time, parameters) = {
        let stream = input
            .streams()
            .find(|stream| stream.parameters().medium() == media::Type::Video)
            .ok_or_else(|| format!("{} has no video stream", snapshot.path().display()))?;
        (
            stream.index(),
            stream.time_base(),
            stream.start_time(),
            stream.parameters(),
        )
    };
    let context = ffmpeg::codec::context::Context::from_parameters(parameters)
        .map_err(|error| error.to_string())?;
    let mut decoder = context
        .decoder()
        .video()
        .map_err(|error| error.to_string())?;
    let width = decoder.width().max(1);
    let height = decoder.height().max(1);
    let mut scaler = ffmpeg::software::scaling::context::Context::get(
        decoder.format(),
        width,
        height,
        format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::flag::Flags::BILINEAR,
    )
    .map_err(|error| error.to_string())?;
    let mut packets = input.packets();
    let mut frames = Vec::new();
    let mut frame_index = 0;
    loop {
        let eof = match packets.find(|(stream, _)| stream.index() == stream_index) {
            Some((_, packet)) => {
                decoder
                    .send_packet(&packet)
                    .map_err(|error| error.to_string())?;
                false
            }
            None => {
                match decoder.send_eof() {
                    Ok(()) | Err(ffmpeg::Error::Eof) => {}
                    Err(error) => return Err(error.to_string()),
                }
                true
            }
        };
        let mut decoded = ffmpeg::frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            let position = frame_time(&decoded, time_base, stream_start_time, frame_index);
            frame_index += 1;
            let mut rgba = ffmpeg::frame::Video::empty();
            scaler
                .run(&decoded, &mut rgba)
                .map_err(|error| error.to_string())?;
            let pixels = tight_rgba_from_rows(rgba.data(0), rgba.stride(0), width, height)?;
            frames.push((
                position,
                VisualFrame::from_rgba_bytes(width, height, pixels)?,
            ));
        }
        if eof {
            break;
        }
    }
    snapshot.verify_current()?;
    if frames.is_empty() {
        return Err(format!(
            "{} contains no GIF frames",
            snapshot.path().display()
        ));
    }
    Ok(DecodedGif {
        frames,
        width,
        height,
    })
}

fn decode_still_image(
    file: &Asset,
    expected_snapshot: AssetSnapshot,
) -> Result<VisualFrame, String> {
    let decoded = decode_still_rgba(file.path())?;
    expected_snapshot.verify_current()?;
    VisualFrame::from_rgba_bytes(decoded.width, decoded.height, decoded.pixels)
}

fn decode_still_rgba(file: &Path) -> Result<DecodedRgba, String> {
    ffmpeg::init().map_err(|error| error.to_string())?;
    let mut input = format::input(file)
        .map_err(|error| format!("could not open {}: {error}", file.display()))?;
    let (stream_index, parameters) = {
        let stream = input
            .streams()
            .find(|stream| stream.parameters().medium() == media::Type::Video)
            .ok_or_else(|| format!("{} has no video stream", file.display()))?;
        (stream.index(), stream.parameters())
    };
    let context = ffmpeg::codec::context::Context::from_parameters(parameters)
        .map_err(|error| error.to_string())?;
    let mut decoder = context
        .decoder()
        .video()
        .map_err(|error| error.to_string())?;
    let width = decoder.width().max(1);
    let height = decoder.height().max(1);
    let mut decoded = ffmpeg::frame::Video::empty();
    let mut received = false;
    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|error| error.to_string())?;
        if decoder.receive_frame(&mut decoded).is_ok() {
            received = true;
            break;
        }
    }
    if !received {
        decoder.send_eof().map_err(|error| error.to_string())?;
        decoder
            .receive_frame(&mut decoded)
            .map_err(|error| error.to_string())?;
    }
    let mut scaler = ffmpeg::software::scaling::context::Context::get(
        decoder.format(),
        width,
        height,
        format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::flag::Flags::BILINEAR,
    )
    .map_err(|error| error.to_string())?;
    let mut rgba = ffmpeg::frame::Video::empty();
    scaler
        .run(&decoded, &mut rgba)
        .map_err(|error| error.to_string())?;
    Ok(DecodedRgba {
        width,
        height,
        pixels: tight_rgba_from_rows(rgba.data(0), rgba.stride(0), width, height)?,
    })
}

fn vectorize_image(
    file: &Path,
    modifier: &VectorizeModifier,
    key: Vec<u8>,
) -> Result<VectorizedOutput, String> {
    let mut decoded = decode_still_rgba(file)?;
    if modifier.color_mode == VectorizeColorMode::BlackAndWhite {
        for pixel in decoded.pixels.chunks_exact_mut(4) {
            let value = if pixel[3] == 0
                || shrimply_math_color::Color::<u8>::from([pixel[0], pixel[1], pixel[2]])
                    .rec709_luma()
                    >= modifier.binary_threshold.min(MAX_BINARY_THRESHOLD) as u8
            {
                255
            } else {
                0
            };
            pixel[..3].fill(value);
        }
    }
    let config = vtracer::Config {
        clustering: match modifier.color_mode {
            VectorizeColorMode::Color => vtracer::Clustering::ColorCluster,
            VectorizeColorMode::BlackAndWhite => vtracer::Clustering::Binary,
        },
        hierarchical: match modifier.hierarchy {
            VectorizeHierarchy::Stacked => vtracer::Hierarchical::Stacked,
            VectorizeHierarchy::Cutout => vtracer::Hierarchical::Cutout,
        },
        filter_speckle: modifier.speckle_size.min(MAX_SPECKLE_SIZE) as usize,
        color_precision: modifier
            .color_precision
            .clamp(MIN_COLOR_PRECISION, MAX_COLOR_PRECISION) as i32,
        layer_difference: modifier.gradient_step.min(MAX_GRADIENT_STEP) as i32,
        mode: match modifier.path_mode {
            VectorizePathMode::Pixel => vtracer::FitMode::Pixel,
            VectorizePathMode::Polygon => vtracer::FitMode::Polygon,
            VectorizePathMode::Spline => vtracer::FitMode::Spline,
        },
        corner_threshold: modifier.corner_threshold_degrees.min(MAX_ANGLE_DEGREES) as i32,
        length_threshold: modifier
            .segment_length
            .clamp(MIN_SEGMENT_LENGTH, MAX_SEGMENT_LENGTH) as f64,
        max_iterations: modifier.max_iterations.min(MAX_ITERATIONS) as usize,
        splice_threshold: modifier.splice_threshold_degrees.min(MAX_ANGLE_DEGREES) as i32,
        path_precision: Some(modifier.path_precision.min(MAX_PATH_PRECISION)),
        ..vtracer::Config::default()
    };
    let image = vtracer::ColorImage {
        pixels: decoded.pixels,
        width: decoded.width as usize,
        height: decoded.height as usize,
    };
    let svg = config
        .build()
        .and_then(|pipeline| pipeline.to_svg(&image))
        .map_err(|error| format!("could not vectorize {}: {error}", file.display()))?;
    Ok(VectorizedOutput {
        key,
        svg,
        width: decoded.width,
        height: decoded.height,
    })
}

fn tight_rgba_from_rows(
    rows: &[u8],
    stride: usize,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, String> {
    let row_bytes = width as usize * 4;
    if stride < row_bytes {
        return Err("RGBA frame stride is smaller than row width".to_string());
    }
    let needed = stride
        .checked_mul(height as usize)
        .ok_or_else(|| "RGBA frame is too large".to_string())?;
    if rows.len() < needed {
        return Err("RGBA frame plane is truncated".to_string());
    }
    if stride == row_bytes {
        return Ok(rows[..needed].to_vec());
    }
    let mut pixels = vec![0; row_bytes * height as usize];
    if pixels.len() >= PARALLEL_RGBA_COPY_MIN_BYTES {
        pixels
            .par_chunks_exact_mut(row_bytes)
            .zip(rows.par_chunks_exact(stride))
            .for_each(|(destination, source)| destination.copy_from_slice(&source[..row_bytes]));
    } else {
        for (destination, source) in pixels
            .chunks_exact_mut(row_bytes)
            .zip(rows.chunks_exact(stride))
        {
            destination.copy_from_slice(&source[..row_bytes]);
        }
    }
    Ok(pixels)
}

fn frame_time(
    frame: &ffmpeg::frame::Video,
    time_base: ffmpeg::Rational,
    stream_start_time: i64,
    frame_index: usize,
) -> Time {
    let Some(timestamp) = frame.timestamp() else {
        return Time::from_fraction(frame_index.min(i64::MAX as usize) as i64, 30);
    };
    let start_time = if stream_start_time > 0 {
        stream_start_time
    } else {
        0
    };
    let timestamp = timestamp.saturating_sub(start_time).max(0);
    let numerator = timestamp.saturating_mul(time_base.numerator() as i64);
    Time::from_fraction(numerator, time_base.denominator() as i64)
}
