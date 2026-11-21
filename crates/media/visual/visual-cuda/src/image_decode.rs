use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use crate::gpu::{CudaVideoCompositor, VisualFrame};
use crate::svg_render::{PreparedSvg, SvgVectorVisualParams, svg_vector_visual};
use crate::visual_source::VisualSourceCache;
use crate::visual_source::{
    VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest,
};
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_drawing_skia::media::{self, animation::Animation};
use shrimply_gpu_cuda_memory::{ResourceKey, global as gpu_memory};
use shrimply_project_document::project::{Time, VideoItem, VideoItemContent};
use shrimply_visual_core::vectorize::{Output as VectorizedOutput, Plan as VectorizePlan};
use uuid::Uuid;

const VECTORIZE_DEBOUNCE: Duration = Duration::from_millis(200);

pub struct ImageDecodeSession {
    file: Asset,
    snapshot: AssetSnapshot,
    width: u32,
    height: u32,
    kind: ImageDecodeKind,
    animation_pending: Option<Receiver<Result<Vec<u8>, String>>>,
    animation: Option<Animation>,
    animation_frame: Option<(Time, Rc<VisualFrame>)>,
    animation_error: Option<String>,
    vectorize_request: Option<VectorizeRequest>,
    vectorize_pending: Option<VectorizePending>,
    vectorized: Option<VectorizedImage>,
    vectorize_error: Option<(Vec<u8>, String)>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ImageDecodeKind {
    Image,
    Animation,
}

struct VectorizeRequest {
    key: Vec<u8>,
    plan: VectorizePlan,
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
            VideoItemContent::Gif | VideoItemContent::AnimatedImage => ImageDecodeKind::Animation,
            _ => return Err("visual media decoder received a non-image item".to_string()),
        };
        Ok(Self {
            file: item.file.clone(),
            snapshot: item.file.snapshot()?,
            width: item.source_width.max(1),
            height: item.source_height.max(1),
            kind,
            animation_pending: None,
            animation: None,
            animation_frame: None,
            animation_error: None,
            vectorize_request: None,
            vectorize_pending: None,
            vectorized: None,
            vectorize_error: None,
        })
    }

    fn request_vectorization(&mut self, item: &VideoItem) -> Result<Option<Vec<u8>>, String> {
        let Some(plan) = VectorizePlan::for_item(item, &self.snapshot.cache_key())? else {
            self.vectorize_request = None;
            return Ok(None);
        };
        let key = plan.key().to_vec();

        if self
            .vectorize_request
            .as_ref()
            .is_none_or(|request| request.key != key)
        {
            self.vectorize_request = Some(VectorizeRequest {
                key: key.clone(),
                plan,
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
            let snapshot = self.snapshot.clone();
            let trace_key = request.key.clone();
            let plan = request.plan.clone();
            let (sender, result) = sync_channel(1);
            self.vectorize_pending = Some(VectorizePending {
                key: trace_key.clone(),
                result,
            });
            rayon::spawn(move || {
                let traced = decode_still_rgba(&snapshot).and_then(|([width, height], pixels)| {
                    plan.vectorize_rgba(pixels, width, height).map_err(|error| {
                        format!("could not vectorize {}: {error}", snapshot.path().display())
                    })
                });
                let _ = sender.send(traced);
            });
            shrimply_profiling::increment("Image vectorization / Prepared requests submitted");
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
                shrimply_project_document::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            ));
        };
        let Some(prepared_svg) = gpu_memory().get_resource(&image.source_key)? else {
            return Ok(VisualRender::Loading(
                shrimply_project_document::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            ));
        };
        let visual = svg_vector_visual(
            SvgVectorVisualParams {
                prepared_svg,
                root_size: shrimply_project_document::project::CanvasSize {
                    width: image.width,
                    height: image.height,
                },
                surface_size: request.render_canvas,
                canvas_size: request.project.canvas_size,
                evaluation: shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
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
            ImageDecodeKind::Animation => self.animation_frame_at(source_position, compositor),
        }
    }

    fn request_animation(&mut self) {
        if self.animation_pending.is_some()
            || self.animation.is_some()
            || self.animation_error.is_some()
        {
            return;
        }
        let snapshot = self.snapshot.clone();
        let (sender, result) = sync_channel(1);
        self.animation_pending = Some(result);
        rayon::spawn(move || {
            let _ = sender.send(snapshot.read_bounded(media::MAX_SOURCE_BYTES));
        });
        shrimply_profiling::increment("animated image decode / Prepared requests submitted");
    }

    fn poll_animation(&mut self) {
        let Some(pending) = &self.animation_pending else {
            return;
        };
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("animated image decoder stopped unexpectedly".to_string())
            }
        };
        self.animation_pending = None;
        match result.and_then(|bytes| Animation::open(&bytes)) {
            Ok(animation) => {
                [self.width, self.height] = animation.dimensions();
                self.animation = Some(animation);
            }
            Err(error) => self.animation_error = Some(error),
        }
    }

    fn animation_frame_at(
        &mut self,
        target: Time,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Option<Rc<VisualFrame>>, String> {
        self.poll_animation();
        if let Some(error) = &self.animation_error {
            return Err(error.clone());
        }
        let Some(animation) = self.animation.as_mut() else {
            self.request_animation();
            return Ok(None);
        };
        let (position, image) = animation.frame_at(target)?;
        if self
            .animation_frame
            .as_ref()
            .is_none_or(|(cached, _)| *cached != position)
        {
            shrimply_profiling::increment("animated image frame GPU residency / Miss");
            let source =
                VisualFrame::from_rgba_bytes(self.width, self.height, media::rgba_pixels(&image)?)?;
            self.animation_frame = Some((position, Rc::new(compositor.upload_frame(&source)?)));
        } else {
            shrimply_profiling::increment("animated image frame GPU residency / Hit");
        }
        let (_, frame) = self
            .animation_frame
            .as_ref()
            .expect("decoded animated image frame is resident");
        compositor.prepare_host_backed_frame(frame, "resident animated image frame")?;
        Ok(Some(frame.clone()))
    }

    fn raster_frame_at(
        &mut self,
        compositor: &mut CudaVideoCompositor,
    ) -> Result<Option<Rc<VisualFrame>>, String> {
        if let Some(frame) = gpu_memory().get_resource::<VisualFrame>(&self.gpu_image_key())? {
            shrimply_profiling::increment("Image GPU residency / Hit");
            compositor.prepare_host_backed_frame(&frame, "cached still image preview")?;
            return Ok(Some(Rc::new((*frame).clone())));
        }
        shrimply_profiling::increment("Image GPU residency / Miss");
        if let Some(frame) = gpu_memory().get_resource::<VisualFrame>(&self.image_key())? {
            return self.upload_still_frame(&frame, compositor).map(Some);
        }
        if gpu_memory().contains_resource(&self.image_key()) {
            return Ok(None);
        }
        let frame = decode_still_image(&self.snapshot)?;
        gpu_memory().insert_resource(self.image_key(), frame.bytes(), frame.clone())?;
        self.upload_still_frame(&frame, compositor).map(Some)
    }
}

impl VisualElement for ImageDecodeSession {
    fn matches(
        &self,
        item: &VideoItem,
        _canvas_size: shrimply_project_document::project::CanvasSize,
    ) -> bool {
        self.file == item.file
            && matches!(
                (self.kind, &item.content),
                (ImageDecodeKind::Image, VideoItemContent::Image)
                    | (
                        ImageDecodeKind::Animation,
                        VideoItemContent::Gif | VideoItemContent::AnimatedImage
                    )
            )
            && self.snapshot.is_current()
    }

    fn prepare(
        &mut self,
        request: VisualPrepareRequest<'_>,
        _track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<(), String> {
        if self.kind == ImageDecodeKind::Animation {
            self.request_animation();
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
            let decoded = decode_still_image(&snapshot);
            let bytes = decoded.as_ref().map_or(0, VisualFrame::bytes);
            if let Err(error) = gpu_memory().finish_resource_load(key, bytes, decoded) {
                tracing::error!(file = %file.display(), %error, "could not finish loading image");
            }
        });
        shrimply_profiling::increment("Image decode / Prepared requests submitted");
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
        let Some(source_position) = shrimply_project_document::project::video_source_time_at(
            request.item,
            request.position,
        ) else {
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
                shrimply_project_document::project::CanvasSize {
                    width: self.width,
                    height: self.height,
                },
            )),
        }
    }
}

fn decode_still_rgba(snapshot: &AssetSnapshot) -> Result<([u32; 2], Vec<u8>), String> {
    let prepared = media::prepare(std::sync::Arc::new(
        snapshot.read_bounded(media::MAX_SOURCE_BYTES)?,
    ))?;
    let image = prepared
        .bitmap()
        .ok_or("still image source is not a bitmap")?;
    Ok((prepared.dimensions(), media::rgba_pixels(image)?))
}

fn decode_still_image(snapshot: &AssetSnapshot) -> Result<VisualFrame, String> {
    let ([width, height], pixels) = decode_still_rgba(snapshot)?;
    VisualFrame::from_rgba_bytes(width, height, pixels)
}
