use hashbrown::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use crate::decode::{DecodeControl, VideoPlane};
use crate::gpu::{CompositedVideoFrame, CudaVideoCompositor, ExportGpuTiming, ExportPixelFormat};
use crate::layer::{GpuFrame, RasterVisual, ResolvedCompositing, VideoLayer, Visual, VisualState};
use crate::visual_source::VisualModifierContext;
pub use crate::visual_source::{Accuracy, CompositeAccuracy};

pub const EXPORT_ASSETS_LOADING: &str = "visual assets are still loading";
use crate::visual_source::{
    GeneratedTransition, VisualElement, VisualPrepareRequest, VisualRender, VisualRenderRequest,
    VisualSourceCache,
};
use shrimply_evaluation::{FrameAudioAnalysis, VisualEvaluation};
use shrimply_evaluation::{
    TransformExpressionCache, resolve, resolve_bool, resolve_item_transform_with_audio,
    resolve_scalar, resolve_vec2,
};
use shrimply_math_core::Fraction;
use shrimply_project::project::{
    AlphaMaskShape, Color, ItemAddress, LayerBlendMode, Project, SequenceReference,
    TextureAddressMode, Time, TransitionSide, VideoItem, VideoSampleMethod, VisualAlphaMask,
    VisualClipTransitionKind, VisualTransitionKind, video_source_time_at,
};
use shrimply_video_modifiers::{ModifierEffect, RasterModifierEffect};
use uuid::Uuid;

macro_rules! abort_render_if_superseded {
    ($control:expr, $action:expr) => {
        if $control.is_some_and(DecodeControl::superseded) {
            $action
        }
    };
}

mod preload;
mod render;
mod sam2;

use render::{FrameItemRenderer, render_project_frame};

const PREVIEW_DISPLAY_SURFACES: usize = 2;

fn manim_source_revision(item: &VideoItem) -> u64 {
    item.file
        .snapshot()
        .map_or(0, |snapshot| snapshot.revision())
}

fn resolve_shape_alpha_mask(
    mask: &VisualAlphaMask,
    evaluation: &VisualEvaluation,
    expressions: &mut TransformExpressionCache,
) -> crate::alpha_mask::ResolvedShapeAlphaMask {
    let size = resolve_vec2(&mask.size, evaluation, expressions).max(glam::Vec2::ZERO);
    crate::alpha_mask::ResolvedShapeAlphaMask {
        center: resolve_vec2(&mask.center, evaluation, expressions),
        size,
        rotation_degrees: resolve_scalar(&mask.rotation_degrees, evaluation, expressions),
        feather: resolve_scalar(&mask.feather, evaluation, expressions).clamp(0.0, 1.0),
        rounding: resolve_scalar(&mask.rounding, evaluation, expressions).clamp(0.0, 1.0),
        shape: match mask.shape {
            AlphaMaskShape::Rectangle => shrimply_render_core::ShapeAlphaMaskKind::Rectangle,
            AlphaMaskShape::Ellipse => shrimply_render_core::ShapeAlphaMaskKind::Ellipse,
            AlphaMaskShape::Polygon => shrimply_render_core::ShapeAlphaMaskKind::Polygon,
        },
        vertices: if mask.shape == AlphaMaskShape::Polygon {
            mask.vertices.iter().map(|point| *point * size).collect()
        } else {
            Vec::new()
        },
        invert: mask.invert,
    }
}

pub enum VideoCommand {
    SetProject {
        project: Arc<Project>,
        revision: u64,
    },
    SetPreviewExclusion(Option<Uuid>),
    ConfigureResources(RenderResourceConfig),
    Render {
        position: Time,
        accuracy: CompositeAccuracy,
    },
    Stop,
}

impl VideoCommand {
    pub fn set_project(project: Arc<Project>, revision: u64) -> Self {
        Self::SetProject { project, revision }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderResourceConfig {
    pub maximum_temporal_decoders: usize,
    pub gpu_host_memory_gib: Fraction,
}

impl Default for RenderResourceConfig {
    fn default() -> Self {
        Self {
            maximum_temporal_decoders: crate::decode::DEFAULT_VIDEO_DECODER_POOL_SIZE,
            gpu_host_memory_gib: Fraction::new_raw(
                shrimply_gpu_memory::default_host_budget_bytes(),
                1024_u64.pow(3),
            ),
        }
    }
}

type PendingProject = Arc<Mutex<Option<(Arc<Project>, u64, u64)>>>;

#[derive(Clone)]
pub struct VideoCommandSender {
    sender: Sender<WorkerCommand>,
    _sam2_claim_waiter: Arc<crate::sam2_analysis::ClaimWaiter>,
    next_request_id: Arc<AtomicU64>,
    cancel_generation: Arc<AtomicU64>,
    last_render: Arc<Mutex<Option<CompositeAccuracy>>>,
    latest_project_generation: Arc<AtomicU64>,
    pending_project: PendingProject,
    project_notification_pending: Arc<AtomicBool>,
    playback_observer: Option<PlaybackRenderObserver>,
}

#[derive(Clone, Copy, Debug)]
pub enum PlaybackRenderEvent {
    Requested {
        request_id: u64,
        position: Time,
    },
    Completed {
        request_id: u64,
        position: Time,
        elapsed: Duration,
        project_fps: Fraction,
    },
}

pub type PlaybackRenderObserver = Arc<dyn Fn(PlaybackRenderEvent) + Send + Sync>;

impl VideoCommandSender {
    pub fn render_generation_is_current(&self, generation: u64) -> bool {
        self.cancel_generation.load(Ordering::Acquire) == generation
    }

    pub fn send(&self, command: VideoCommand) -> Result<(), mpsc::SendError<Box<VideoCommand>>> {
        let worker_command = match command {
            VideoCommand::SetProject { project, revision } => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                self.last_render
                    .lock()
                    .expect("last video render mutex poisoned")
                    .take();
                let mut pending = self
                    .pending_project
                    .lock()
                    .expect("pending video project mutex poisoned");
                let project_generation = self
                    .latest_project_generation
                    .fetch_add(1, Ordering::AcqRel)
                    + 1;
                let previous = pending.replace((project, revision, project_generation));
                if self
                    .project_notification_pending
                    .swap(true, Ordering::AcqRel)
                {
                    drop(pending);
                    drop(previous);
                    return Ok(());
                }
                if self.sender.send(WorkerCommand::SetProject).is_err() {
                    self.project_notification_pending
                        .store(false, Ordering::Release);
                    let (project, revision, _) = pending
                        .take()
                        .expect("pending video project disappeared while sending notification");
                    *pending = previous;
                    return Err(mpsc::SendError(Box::new(VideoCommand::SetProject {
                        project,
                        revision,
                    })));
                }
                drop(pending);
                drop(previous);
                return Ok(());
            }
            VideoCommand::SetPreviewExclusion(item_id) => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::SetPreviewExclusion(item_id)
            }
            VideoCommand::Render { position, accuracy } => {
                let request_id = self.next_request_id.fetch_add(1, Ordering::AcqRel) + 1;
                let mut previous = self
                    .last_render
                    .lock()
                    .expect("last video render mutex poisoned");
                let continuous = previous.is_some_and(|previous| previous == accuracy)
                    && accuracy.continuous_playback();
                if !continuous {
                    self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                }
                *previous = Some(accuracy);
                let cancel_generation = self.cancel_generation.load(Ordering::Acquire);
                if accuracy.continuous_playback()
                    && let Some(observer) = &self.playback_observer
                {
                    observer(PlaybackRenderEvent::Requested {
                        request_id,
                        position,
                    });
                }
                WorkerCommand::Render {
                    position,
                    accuracy,
                    request_id,
                    cancel_generation,
                }
            }
            VideoCommand::ConfigureResources(config) => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::ConfigureResources(config)
            }
            VideoCommand::Stop => {
                self.cancel_generation.fetch_add(1, Ordering::AcqRel);
                WorkerCommand::Stop
            }
        };
        self.sender.send(worker_command).map_err(|error| {
            mpsc::SendError(Box::new(match error.0 {
                WorkerCommand::SetProject => {
                    unreachable!("project notifications return before common send handling")
                }
                WorkerCommand::SetPreviewExclusion(item_id) => {
                    VideoCommand::SetPreviewExclusion(item_id)
                }
                WorkerCommand::Render {
                    position, accuracy, ..
                } => VideoCommand::Render { position, accuracy },
                WorkerCommand::ConfigureResources(config) => {
                    VideoCommand::ConfigureResources(config)
                }
                WorkerCommand::ScheduleSam2Analysis => {
                    unreachable!("SAM2 scheduling is an internal compositor command")
                }
                WorkerCommand::Stop => VideoCommand::Stop,
            }))
        })
    }
}

enum WorkerCommand {
    SetProject,
    SetPreviewExclusion(Option<Uuid>),
    ConfigureResources(RenderResourceConfig),
    ScheduleSam2Analysis,
    Render {
        position: Time,
        accuracy: CompositeAccuracy,
        request_id: u64,
        cancel_generation: u64,
    },
    Stop,
}

pub enum VideoEvent {
    Loading {
        position: Time,
        show_spinner: bool,
        render_elapsed: Duration,
        render_generation: u64,
    },
    Frame {
        frame: CompositedVideoFrame,
        audio_analysis: FrameAudioAnalysis,
        position: Time,
        revision: u64,
        excluded_item_id: Option<Uuid>,
        settled: bool,
        render_elapsed: Duration,
        render_generation: u64,
    },
    Clear {
        audio_analysis: FrameAudioAnalysis,
        position: Time,
        revision: u64,
        excluded_item_id: Option<Uuid>,
        render_elapsed: Duration,
        render_generation: u64,
    },
    ManimDuration {
        item_id: Uuid,
        source_revision: u64,
        duration: Time,
    },
    ManimParameters {
        item_id: Uuid,
        source_revision: u64,
        scene: String,
        parameters: Vec<shrimply_project::project::ManimParameter>,
        render_is_current: bool,
    },
    ManimStatus {
        item_id: Uuid,
        source_revision: u64,
        error: Option<String>,
    },
    Error(String),
}

fn publish_render_event(
    event_tx: &SyncSender<VideoEvent>,
    event: VideoEvent,
    accuracy: CompositeAccuracy,
) {
    if accuracy.continuous_playback() {
        let _ = event_tx.try_send(event);
    } else {
        let _ = event_tx.send(event);
    }
}

struct RenderedFrame {
    frame: Option<CompositedVideoFrame>,
    audio_analysis: FrameAudioAnalysis,
    loading: bool,
    loading_placeholder: bool,
    clear: bool,
    errors: Vec<String>,
    manim_durations: Vec<(Uuid, u64, Time)>,
    manim_parameters: Vec<(
        Uuid,
        u64,
        String,
        Vec<shrimply_project::project::ManimParameter>,
        bool,
    )>,
    manim_statuses: Vec<(Uuid, u64, Option<String>)>,
    superseded: bool,
}

pub struct VideoExportRenderer {
    render_cache: RenderCache,
    sessions: RenderSessions,
    compositor: CudaVideoCompositor,
    prepare_active_sources: bool,
}

struct RenderSessions {
    elements: HashMap<VisualElementKey, Box<dyn VisualElement>>,
    decoders: crate::decode::VideoDecoderPool,
    sources: VisualSourceCache,
    volume: shrimply_audio::streaming::FrameVolumeSampler,
    mouth: shrimply_audio::streaming::FrameMouthSampler,
    audio_sample_rate: u32,
    export: bool,
    resource_config: RenderResourceConfig,
    volume_revision: u64,
}

#[derive(Clone, Copy)]
struct SourcePrepareRequest<'a> {
    sequence_path: &'a [Uuid],
    track_id: Uuid,
    item: &'a VideoItem,
    position: Time,
    canvas_size: shrimply_project::project::CanvasSize,
    accuracy: CompositeAccuracy,
    decode_control: Option<&'a DecodeControl>,
    route: VideoDecodeRoute,
    prefetch: bool,
}

#[derive(Clone, Copy)]
struct VideoDecodeRoute {
    plane: VideoPlane,
    handoff_item_id: Option<Uuid>,
}

#[derive(Clone, Copy, Default)]
struct VideoDecodeRoutes {
    color_handoff_item_id: Option<Uuid>,
    alpha_handoff_item_id: Option<Uuid>,
}

impl VideoDecodeRoutes {
    fn route(self, plane: VideoPlane) -> VideoDecodeRoute {
        VideoDecodeRoute {
            plane,
            handoff_item_id: match plane {
                VideoPlane::Color => self.color_handoff_item_id,
                VideoPlane::Alpha => self.alpha_handoff_item_id,
            },
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum VisualElementKey {
    Item {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
        item_id: Uuid,
        media_track_id: u32,
        plane: VideoPlane,
    },
    Manim {
        sequence_path: Vec<Uuid>,
        track_id: Uuid,
        item_id: Uuid,
        width: u32,
        height: u32,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ScopedTrackKey {
    sequence_path: Vec<Uuid>,
    track_id: Uuid,
}

impl Default for RenderSessions {
    fn default() -> Self {
        Self::new(
            shrimply_audio::streaming::EXPRESSION_SAMPLE_RATE_HZ,
            false,
            RenderResourceConfig::default(),
        )
    }
}

impl RenderSessions {
    fn new(audio_sample_rate: u32, export: bool, resource_config: RenderResourceConfig) -> Self {
        shrimply_gpu_memory::configure(shrimply_math_media::gib_to_bytes(
            resource_config.gpu_host_memory_gib,
        ));
        Self {
            elements: HashMap::new(),
            decoders: crate::decode::VideoDecoderPool::new(
                resource_config.maximum_temporal_decoders,
            ),
            sources: VisualSourceCache::default(),
            volume: shrimply_audio::streaming::FrameVolumeSampler::new(audio_sample_rate),
            mouth: if export {
                shrimply_audio::streaming::FrameMouthSampler::export()
            } else {
                shrimply_audio::streaming::FrameMouthSampler::preview()
            },
            audio_sample_rate,
            export,
            resource_config,
            volume_revision: 0,
        }
    }

    fn clear(&mut self) {
        self.elements.clear();
        self.decoders =
            crate::decode::VideoDecoderPool::new(self.resource_config.maximum_temporal_decoders);
        self.sources.clear();
        self.volume = shrimply_audio::streaming::FrameVolumeSampler::new(self.audio_sample_rate);
        self.mouth = if self.export {
            shrimply_audio::streaming::FrameMouthSampler::export()
        } else {
            shrimply_audio::streaming::FrameMouthSampler::preview()
        };
    }

    fn configure_resources(&mut self, config: RenderResourceConfig) {
        assert!(
            config.maximum_temporal_decoders > 0,
            "temporal decoder pool cannot be empty"
        );
        self.resource_config = config;
        self.decoders.configure(config.maximum_temporal_decoders);
        shrimply_gpu_memory::configure(shrimply_math_media::gib_to_bytes(
            config.gpu_host_memory_gib,
        ));
    }

    fn remove_manim_replacement(&mut self, replacement: &VisualElementKey) {
        let VisualElementKey::Manim {
            sequence_path: replacement_path,
            track_id: replacement_track,
            item_id: replacement_item,
            ..
        } = replacement
        else {
            return;
        };
        self.elements.retain(|key, _| {
            let replace = matches!(
                key,
                VisualElementKey::Manim {
                    sequence_path,
                    track_id,
                    item_id,
                    ..
                } if sequence_path == replacement_path
                    && track_id == replacement_track
                    && item_id == replacement_item
            );
            if replace
                && let VisualElementKey::Manim {
                    item_id,
                    width,
                    height,
                    ..
                } = key
            {
                tracing::debug!(
                    item = %item_id,
                    width,
                    height,
                    "stopping replaced Manim render session",
                );
            }
            !replace
        });
    }

    fn create_element(
        &mut self,
        sequence_path: &[Uuid],
        track_id: Uuid,
        item: &VideoItem,
        canvas_size: shrimply_project::project::CanvasSize,
        route: VideoDecodeRoute,
    ) -> Result<Box<dyn VisualElement>, String> {
        if !matches!(
            item.content,
            shrimply_project::project::VideoItemContent::Media
        ) {
            return crate::visual_source::create_renderer(item, canvas_size);
        }
        let owner = self
            .decoders
            .owner(sequence_path, track_id, item.id, route.plane);
        let handoff_from = route.handoff_item_id.map(|item_id| {
            self.decoders
                .owner(sequence_path, track_id, item_id, route.plane)
        });
        let decoder = self.decoders.decoder(item, owner)?;
        Ok(Box::new(crate::decode::VideoElement::new(
            decoder,
            handoff_from,
        )))
    }

    fn prepare_source(&mut self, request: SourcePrepareRequest<'_>) -> Result<(), String> {
        let SourcePrepareRequest {
            sequence_path,
            track_id,
            item,
            position,
            canvas_size,
            accuracy,
            decode_control,
            route,
            prefetch,
        } = request;
        let key = VisualElementKey::Item {
            sequence_path: sequence_path.to_vec(),
            track_id,
            item_id: item.id,
            media_track_id: item.track_id,
            plane: route.plane,
        };
        let create = self
            .elements
            .get(&key)
            .is_none_or(|element| !element.matches(item, canvas_size));
        if create {
            let element = self.create_element(sequence_path, track_id, item, canvas_size, route)?;
            self.elements.insert(key.clone(), element);
        }
        self.elements
            .get_mut(&key)
            .expect("prepared visual element was just created")
            .prepare(
                VisualPrepareRequest {
                    item,
                    position,
                    accuracy,
                    decode_control,
                    prefetch,
                },
                track_id,
                &mut self.sources,
            )
    }
}

#[derive(Default)]
struct RenderCache {
    expressions: TransformExpressionCache,
    morphs: HashMap<MorphCacheKey, Rc<CachedMorph>>,
    transparent_fill_keys: HashMap<(ItemAddress, Uuid, u64), String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MorphCacheKey {
    sequence_path: Vec<Uuid>,
    track_id: Uuid,
    outgoing_id: Uuid,
    incoming_id: Uuid,
    width: u32,
    height: u32,
    content_hash: u64,
}

enum CachedMorph {
    Vector {
        morph: Rc<crate::vector_morph::PreparedVectorMorph>,
        source_state: VisualState,
        target_state: VisualState,
    },
    OpticalFlow {
        source: Rc<crate::gpu::VisualFrame>,
        target: Rc<crate::gpu::VisualFrame>,
        flow: shrimply_nvidia_optical_flow::FlowField,
        source_compositing: ResolvedCompositing,
        target_compositing: ResolvedCompositing,
        source_strategy: shrimply_project::project::SkiaDrawingStrategy,
        target_strategy: shrimply_project::project::SkiaDrawingStrategy,
    },
}

use shrimply_video_core::clip_transition::{ActiveClipTransition, ClipTransitionRole};
use shrimply_video_core::sequence::{ActiveVideoItem, active_video_items};

fn solid_video_layer(
    compositor: &mut CudaVideoCompositor,
    canvas_size: shrimply_project::project::CanvasSize,
    color: Color<u8>,
    opacity: f32,
) -> Result<VideoLayer, String> {
    Ok(VideoLayer::Rgba {
        layer: compositor.solid_layer(canvas_size, color)?,
        transform: shrimply_math_geometry::ComposedTransform2D::IDENTITY,
        motion_blur: None,
        sample_method: VideoSampleMethod::Nearest,
        compositing: ResolvedCompositing {
            opacity,
            blend_mode: LayerBlendMode::Normal,
        },
        crop: [0.0; 4],
        padding: [0.0; 4],
        address_mode: TextureAddressMode::Transparent,
    })
}

#[derive(Clone, Copy)]
enum RenderMode {
    Preview {
        accuracy: CompositeAccuracy,
    },
    ExportContentAccurate {
        background_alpha: u8,
        prepare_active_sources: bool,
    },
}

impl RenderMode {
    const fn accuracy(self) -> CompositeAccuracy {
        match self {
            Self::Preview { accuracy } => accuracy,
            Self::ExportContentAccurate { .. } => CompositeAccuracy::FULLY_ACCURATE,
        }
    }

    const fn prepare_active_sources(self) -> bool {
        match self {
            Self::Preview { .. } => true,
            Self::ExportContentAccurate {
                prepare_active_sources,
                ..
            } => prepare_active_sources,
        }
    }
}

impl VideoExportRenderer {
    pub fn new(audio_sample_rate: u32) -> Result<Self, String> {
        Self::new_with_resources(audio_sample_rate, RenderResourceConfig::default())
    }

    pub fn new_with_resources(
        audio_sample_rate: u32,
        resource_config: RenderResourceConfig,
    ) -> Result<Self, String> {
        Ok(Self {
            render_cache: RenderCache::default(),
            sessions: RenderSessions::new(audio_sample_rate, true, resource_config),
            compositor: CudaVideoCompositor::new()?,
            prepare_active_sources: true,
        })
    }

    pub fn set_prepare_active_sources(&mut self, prepare_active_sources: bool) {
        self.prepare_active_sources = prepare_active_sources;
    }

    pub fn render(
        &mut self,
        project: &Project,
        position: Time,
        background_alpha: u8,
    ) -> Result<CompositedVideoFrame, String> {
        self.render_items_inner(project, position, background_alpha, None, None, false)
    }

    pub fn render_items(
        &mut self,
        project: &Project,
        position: Time,
        background_alpha: u8,
        item_ids: &[Uuid],
    ) -> Result<CompositedVideoFrame, String> {
        if item_ids.is_empty() {
            return Err("no visual items were selected".to_string());
        }
        self.render_items_inner(
            project,
            position,
            background_alpha,
            Some(item_ids),
            None,
            false,
        )
    }

    pub(crate) fn render_cache_item(
        &mut self,
        project: &Project,
        position: Time,
        address: &ItemAddress,
    ) -> Result<CompositedVideoFrame, String> {
        self.render_cache_item_inner(project, position, address, false)
    }

    pub(crate) fn render_transparent_fill_input(
        &mut self,
        project: &Project,
        position: Time,
        address: &ItemAddress,
    ) -> Result<CompositedVideoFrame, String> {
        self.render_cache_item_inner(project, position, address, true)
    }

    fn render_cache_item_inner(
        &mut self,
        project: &Project,
        position: Time,
        address: &ItemAddress,
        snap_cache_item: bool,
    ) -> Result<CompositedVideoFrame, String> {
        let ItemAddress::Video {
            sequence_path,
            item_id,
            ..
        } = address
        else {
            return Err("visual cache requires a video item address".to_string());
        };
        let root_item_id = sequence_path.first().unwrap_or(item_id);
        self.render_items_inner(
            project,
            position,
            0,
            Some(std::slice::from_ref(root_item_id)),
            Some(address),
            snap_cache_item,
        )
    }

    fn render_items_inner(
        &mut self,
        project: &Project,
        position: Time,
        background_alpha: u8,
        item_ids: Option<&[Uuid]>,
        cache_item: Option<&ItemAddress>,
        snap_cache_item: bool,
    ) -> Result<CompositedVideoFrame, String> {
        let volume_revision = self.sessions.volume_revision;
        let audio_analysis = FrameAudioAnalysis {
            volume: self
                .sessions
                .volume
                .sample(project, position, volume_revision),
            mouth: self
                .sessions
                .mouth
                .sample(project, position, volume_revision),
        };
        let mut rendered = render_project_frame(
            project,
            position,
            &mut self.sessions,
            &mut self.render_cache,
            &mut self.compositor,
            RenderMode::ExportContentAccurate {
                background_alpha,
                prepare_active_sources: self.prepare_active_sources,
            },
            &audio_analysis,
            item_ids,
            cache_item,
            snap_cache_item,
            None,
            None,
        );
        if rendered
            .errors
            .iter()
            .any(|error| crate::decode::is_decoder_startup_pressure(error))
        {
            let _ = crate::decode::take_decoder_pressure();
            self.sessions.decoders.reclaim_idle();
            match self
                .compositor
                .relieve_all_gpu_pressure("export video decoder startup retry")
            {
                Ok(()) => {
                    shrimply_benchmarking::increment(
                        "Temporal decoder / Export starts retried after GPU relief",
                    );
                    rendered = render_project_frame(
                        project,
                        position,
                        &mut self.sessions,
                        &mut self.render_cache,
                        &mut self.compositor,
                        RenderMode::ExportContentAccurate {
                            background_alpha,
                            prepare_active_sources: self.prepare_active_sources,
                        },
                        &audio_analysis,
                        item_ids,
                        cache_item,
                        snap_cache_item,
                        None,
                        None,
                    );
                }
                Err(error) => rendered.errors.push(format!(
                    "Could not collect GPU garbage for export video decoder retry: {error}"
                )),
            }
        }
        if rendered.errors.is_empty() {
            match rendered.frame {
                Some(frame) => Ok(frame),
                None => self
                    .compositor
                    .render_export(project.canvas_size, &[], background_alpha),
            }
        } else {
            let error = rendered.errors.join("\n");
            tracing::error!(
                "video_compositor: export render failed position={} errors={}",
                position.as_label(),
                error.replace('\n', " | "),
            );
            Err(error)
        }
    }

    pub fn copy_to_hw_frame(
        &mut self,
        source: CompositedVideoFrame,
        destination: &mut ffmpeg_next::frame::Video,
        pixel_format: ExportPixelFormat,
    ) -> Result<ExportGpuTiming, String> {
        self.compositor
            .copy_to_ffmpeg_hw_frame_and_recycle(source, destination, pixel_format)
    }

    pub fn copy_to_rgba_frame(
        &mut self,
        source: CompositedVideoFrame,
        destination: &mut ffmpeg_next::frame::Video,
    ) -> Result<ExportGpuTiming, String> {
        self.compositor
            .copy_to_ffmpeg_rgba_frame_and_recycle(source, destination)
    }

    pub fn decoder_session_count(&self) -> usize {
        self.sessions.decoders.session_count()
    }

    pub fn shutdown(&mut self) {
        self.render_cache = RenderCache::default();
        self.sessions.clear();
    }
}

impl Drop for VideoExportRenderer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn spawn_worker(project: Project) -> (VideoCommandSender, Receiver<VideoEvent>) {
    spawn_worker_with_resources(project, RenderResourceConfig::default())
}

pub fn spawn_worker_with_resources(
    project: Project,
    resource_config: RenderResourceConfig,
) -> (VideoCommandSender, Receiver<VideoEvent>) {
    spawn_worker_with_resources_and_observer(project, resource_config, None)
}

pub fn spawn_worker_with_resources_and_observer(
    project: Project,
    resource_config: RenderResourceConfig,
    playback_observer: Option<PlaybackRenderObserver>,
) -> (VideoCommandSender, Receiver<VideoEvent>) {
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::sync_channel(PREVIEW_DISPLAY_SURFACES);
    let next_request_id = Arc::new(AtomicU64::new(0));
    let cancel_generation = Arc::new(AtomicU64::new(0));
    let last_render = Arc::new(Mutex::new(None));
    let latest_project_generation = Arc::new(AtomicU64::new(0));
    let pending_project = Arc::new(Mutex::new(None));
    let project_notification_pending = Arc::new(AtomicBool::new(false));
    let sam2_claim_waiter = Arc::new(crate::sam2_analysis::ClaimWaiter::new({
        let command_tx = command_tx.clone();
        move || {
            let _ = command_tx.send(WorkerCommand::ScheduleSam2Analysis);
        }
    }));
    let worker_cancel_generation = cancel_generation.clone();
    let worker_project_generation = latest_project_generation.clone();
    let worker_project = pending_project.clone();
    let worker_project_notification = project_notification_pending.clone();
    let worker_playback_observer = playback_observer.clone();
    let worker_sam2_claim_waiter = Arc::downgrade(&sam2_claim_waiter);
    thread::spawn(move || {
        video_compositor_worker(
            project,
            resource_config,
            VideoWorkerChannels {
                command_rx,
                sam2_claim_waiter: worker_sam2_claim_waiter,
                event_tx,
                cancel_generation: worker_cancel_generation,
                latest_project_generation: worker_project_generation,
                pending_project: worker_project,
                project_notification_pending: worker_project_notification,
                playback_observer: worker_playback_observer,
            },
        )
    });
    (
        VideoCommandSender {
            sender: command_tx,
            _sam2_claim_waiter: sam2_claim_waiter,
            next_request_id,
            cancel_generation,
            last_render,
            latest_project_generation,
            pending_project,
            project_notification_pending,
            playback_observer,
        },
        event_rx,
    )
}

struct VideoWorkerChannels {
    command_rx: Receiver<WorkerCommand>,
    sam2_claim_waiter: Weak<crate::sam2_analysis::ClaimWaiter>,
    event_tx: SyncSender<VideoEvent>,
    cancel_generation: Arc<AtomicU64>,
    latest_project_generation: Arc<AtomicU64>,
    pending_project: PendingProject,
    project_notification_pending: Arc<AtomicBool>,
    playback_observer: Option<PlaybackRenderObserver>,
}

fn video_compositor_worker(
    mut project: Project,
    resource_config: RenderResourceConfig,
    channels: VideoWorkerChannels,
) {
    let VideoWorkerChannels {
        command_rx,
        sam2_claim_waiter,
        event_tx,
        cancel_generation,
        latest_project_generation,
        pending_project,
        project_notification_pending,
        playback_observer,
    } = channels;
    let _pending_project_reset = PendingProjectReset {
        project: pending_project.clone(),
        notification: project_notification_pending.clone(),
    };
    let mut compositor = None;
    let mut sessions = RenderSessions::new(
        shrimply_audio::streaming::EXPRESSION_SAMPLE_RATE_HZ,
        false,
        resource_config,
    );
    let mut render_cache = RenderCache::default();
    let mut last_errors = Vec::new();
    let mut sam2_analyses = HashMap::new();
    let mut project_revision = 0;
    let mut project_generation = 0;
    let mut preview_exclusion = None;
    loop {
        let Ok(command) = command_rx.recv() else {
            return;
        };
        match command {
            WorkerCommand::SetProject => {
                let _span = tracing::debug_span!("video_compositor.set_project").entered();
                let _measurement = shrimply_benchmarking::measure("Video / Project update");
                if let Some((next_project, next_revision, next_generation)) =
                    take_pending_project(&pending_project, &project_notification_pending)
                {
                    project = (*next_project).clone();
                    project_revision = next_revision;
                    project_generation = next_generation;
                    render_cache.morphs.clear();
                    render_cache.transparent_fill_keys.clear();
                    let _measurement =
                        shrimply_benchmarking::measure("Video / Retain project sessions");
                    retain_project_sessions(&project, &mut sessions);
                    schedule_sam2_analysis(
                        &project,
                        &mut sam2_analyses,
                        &event_tx,
                        &sam2_claim_waiter,
                    );
                }
            }
            WorkerCommand::SetPreviewExclusion(item_id) => preview_exclusion = item_id,
            WorkerCommand::ConfigureResources(config) => sessions.configure_resources(config),
            WorkerCommand::ScheduleSam2Analysis => {
                if let Some(waiter) = sam2_claim_waiter.upgrade() {
                    waiter.consume_notification();
                }
                schedule_sam2_analysis(&project, &mut sam2_analyses, &event_tx, &sam2_claim_waiter);
            }
            WorkerCommand::Render {
                position,
                accuracy,
                request_id,
                cancel_generation: render_generation,
            } => {
                let coalesced = {
                    let _measurement = shrimply_benchmarking::measure("Video / Coalesce commands");
                    coalesce_pending_commands(
                        position,
                        accuracy,
                        request_id,
                        render_generation,
                        &mut project,
                        &mut project_revision,
                        &mut project_generation,
                        &mut preview_exclusion,
                        &mut sessions,
                        &mut render_cache,
                        &command_rx,
                        &pending_project,
                        &project_notification_pending,
                        &sam2_claim_waiter,
                    )
                };
                let Some((position, accuracy, request_id, render_generation)) = coalesced else {
                    return;
                };
                schedule_sam2_analysis(&project, &mut sam2_analyses, &event_tx, &sam2_claim_waiter);
                let render_project_generation = project_generation;
                let decode_control =
                    DecodeControl::new(render_generation, cancel_generation.clone());
                if decode_control.superseded() {
                    continue;
                }
                let _span = tracing::debug_span!(
                    "video_compositor.render",
                    position = %position.as_label(),
                    ?accuracy,
                )
                .entered();
                if compositor.is_none() {
                    let _measurement =
                        shrimply_benchmarking::measure("Video / Compositor initialization");
                    match CudaVideoCompositor::new() {
                        Ok(next) => {
                            compositor = Some(next);
                            tracing::debug!("CUDA compositor initialized");
                        }
                        Err(error) => {
                            tracing::error!(%error, "could not initialize the CUDA compositor");
                            if let Err(send_error) = event_tx.try_send(VideoEvent::Error(error)) {
                                tracing::error!(
                                    %send_error,
                                    "could not publish the CUDA compositor initialization error",
                                );
                            }
                            continue;
                        }
                    }
                }

                let Some(compositor) = compositor.as_mut() else {
                    continue;
                };
                let render_started = Instant::now();
                let _measurement = shrimply_benchmarking::measure("Video / Render request");
                let audio_analysis = {
                    let _measurement = shrimply_benchmarking::measure("Video / Volume sampling");
                    let volume_revision = sessions.volume_revision;
                    FrameAudioAnalysis {
                        volume: sessions.volume.sample(&project, position, volume_revision),
                        mouth: sessions.mouth.sample(&project, position, volume_revision),
                    }
                };
                let mut rendered = render_project_frame(
                    &project,
                    position,
                    &mut sessions,
                    &mut render_cache,
                    compositor,
                    RenderMode::Preview { accuracy },
                    &audio_analysis,
                    None,
                    None,
                    false,
                    preview_exclusion,
                    Some(&decode_control),
                );
                if accuracy.time_accurate()
                    && !accuracy.continuous_playback()
                    && rendered
                        .errors
                        .iter()
                        .any(|error| crate::decode::is_decoder_startup_pressure(error))
                    && !decode_control.superseded()
                {
                    compositor.set_render_control(Some(decode_control.clone()));
                    let _ = crate::decode::take_decoder_pressure();
                    sessions.decoders.reclaim_idle();
                    match compositor.relieve_all_gpu_pressure("video decoder startup retry") {
                        Ok(()) if !decode_control.superseded() => {
                            shrimply_benchmarking::increment(
                                "Temporal decoder / Accurate starts retried after GPU relief",
                            );
                            rendered = render_project_frame(
                                &project,
                                position,
                                &mut sessions,
                                &mut render_cache,
                                compositor,
                                RenderMode::Preview { accuracy },
                                &audio_analysis,
                                None,
                                None,
                                false,
                                preview_exclusion,
                                Some(&decode_control),
                            );
                        }
                        Ok(()) => {}
                        Err(error) => rendered.errors.push(format!(
                            "Could not relieve GPU pressure for video decoder retry: {error}"
                        )),
                    }
                    compositor.set_render_control(None);
                }
                let render_elapsed = render_started.elapsed();
                // Parameter reflection is consumed from the Manim renderer only once. A newer
                // preview request (for example, deselecting the item while it first loads) may
                // supersede this frame after that happens, so publish the metadata independently
                // of the frame. The UI validates the source revision, scene, and current values.
                for (item_id, source_revision, scene, parameters, render_is_current) in
                    &rendered.manim_parameters
                {
                    if event_tx
                        .send(VideoEvent::ManimParameters {
                            item_id: *item_id,
                            source_revision: *source_revision,
                            scene: scene.clone(),
                            parameters: parameters.clone(),
                            render_is_current: *render_is_current,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                if latest_project_generation.load(Ordering::Acquire) != render_project_generation {
                    shrimply_benchmarking::increment(
                        "Video / Project-superseded frames not published",
                    );
                    continue;
                }
                if rendered.superseded || decode_control.superseded() {
                    shrimply_benchmarking::increment("Video / Superseded frames not published");
                    continue;
                }
                // Duration is a one-shot state update. Use backpressure so a full frame queue
                // cannot silently discard it forever, but publish only from the current render.
                for (item_id, source_revision, duration) in &rendered.manim_durations {
                    if event_tx
                        .send(VideoEvent::ManimDuration {
                            item_id: *item_id,
                            source_revision: *source_revision,
                            duration: *duration,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                if accuracy.continuous_playback()
                    && let Some(observer) = &playback_observer
                {
                    observer(PlaybackRenderEvent::Completed {
                        request_id,
                        position,
                        elapsed: render_elapsed,
                        project_fps: project.fps,
                    });
                }
                let errors_changed = rendered.errors != last_errors;
                for (item_id, source_revision, error) in rendered.manim_statuses {
                    let _ = event_tx.try_send(VideoEvent::ManimStatus {
                        item_id,
                        source_revision,
                        error,
                    });
                }
                if errors_changed {
                    for error in &rendered.errors {
                        tracing::error!(
                            position = %position.as_label(),
                            ?accuracy,
                            %error,
                            "video frame rendering failed",
                        );
                        if matches!(
                            event_tx.try_send(VideoEvent::Error(error.clone())),
                            Err(mpsc::TrySendError::Disconnected(_))
                        ) {
                            return;
                        }
                    }
                    last_errors = rendered.errors.clone();
                }
                match rendered.frame {
                    Some(frame) => {
                        publish_render_event(
                            &event_tx,
                            VideoEvent::Frame {
                                frame,
                                audio_analysis: rendered.audio_analysis,
                                position,
                                revision: project_revision,
                                excluded_item_id: preview_exclusion,
                                settled: (accuracy.content_accurate()
                                    || (accuracy.time_accurate() && !accuracy.local_scrub()))
                                    && !rendered.loading,
                                render_elapsed,
                                render_generation,
                            },
                            accuracy,
                        );
                        if rendered.loading {
                            publish_render_event(
                                &event_tx,
                                VideoEvent::Loading {
                                    position,
                                    show_spinner: !rendered.loading_placeholder,
                                    render_elapsed,
                                    render_generation,
                                },
                                accuracy,
                            );
                        }
                    }
                    None if rendered.loading => {
                        publish_render_event(
                            &event_tx,
                            VideoEvent::Loading {
                                position,
                                show_spinner: !rendered.loading_placeholder,
                                render_elapsed,
                                render_generation,
                            },
                            accuracy,
                        );
                    }
                    None if rendered.clear => {
                        publish_render_event(
                            &event_tx,
                            VideoEvent::Clear {
                                audio_analysis: rendered.audio_analysis,
                                position,
                                revision: project_revision,
                                excluded_item_id: preview_exclusion,
                                render_elapsed,
                                render_generation,
                            },
                            accuracy,
                        );
                    }
                    None => {}
                }
                if (accuracy.continuous_playback() || accuracy.local_scrub())
                    && let Some(startup_bytes) = crate::decode::take_decoder_pressure()
                {
                    compositor.set_render_control(Some(decode_control.clone()));
                    sessions.decoders.reclaim_idle();
                    if let Err(error) = compositor.relieve_decoder_gpu_pressure(startup_bytes) {
                        tracing::warn!(
                            %error,
                            startup_bytes,
                            "could not relieve speculative video decoder GPU pressure",
                        );
                    }
                    compositor.set_render_control(None);
                }
            }
            WorkerCommand::Stop => return,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn coalesce_pending_commands(
    mut position: Time,
    mut accuracy: CompositeAccuracy,
    mut request_id: u64,
    mut render_generation: u64,
    project: &mut Project,
    project_revision: &mut u64,
    project_generation: &mut u64,
    preview_exclusion: &mut Option<Uuid>,
    sessions: &mut RenderSessions,
    render_cache: &mut RenderCache,
    command_rx: &Receiver<WorkerCommand>,
    pending_project: &Mutex<Option<(Arc<Project>, u64, u64)>>,
    project_notification_pending: &AtomicBool,
    sam2_claim_waiter: &Weak<crate::sam2_analysis::ClaimWaiter>,
) -> Option<(Time, CompositeAccuracy, u64, u64)> {
    while let Ok(next) = command_rx.try_recv() {
        match next {
            WorkerCommand::SetProject => {
                if let Some((next_project, next_revision, next_generation)) =
                    take_pending_project(pending_project, project_notification_pending)
                {
                    *project = (*next_project).clone();
                    *project_revision = next_revision;
                    *project_generation = next_generation;
                    render_cache.morphs.clear();
                    render_cache.transparent_fill_keys.clear();
                    retain_project_sessions(project, sessions);
                }
            }
            WorkerCommand::SetPreviewExclusion(item_id) => *preview_exclusion = item_id,
            WorkerCommand::ConfigureResources(config) => sessions.configure_resources(config),
            WorkerCommand::ScheduleSam2Analysis => {
                if let Some(waiter) = sam2_claim_waiter.upgrade() {
                    waiter.consume_notification();
                }
            }
            WorkerCommand::Render {
                position: next_position,
                accuracy: next_accuracy,
                request_id: next_request_id,
                cancel_generation: next_render_generation,
            } => {
                position = next_position;
                accuracy = next_accuracy;
                request_id = next_request_id;
                render_generation = next_render_generation;
            }
            WorkerCommand::Stop => return None,
        }
    }
    Some((position, accuracy, request_id, render_generation))
}

fn schedule_sam2_analysis(
    project: &Project,
    scheduled: &mut HashMap<Uuid, crate::sam2_analysis::RunId>,
    event_tx: &SyncSender<VideoEvent>,
    claim_waiter: &Weak<crate::sam2_analysis::ClaimWaiter>,
) {
    let Some(claim_waiter) = claim_waiter.upgrade() else {
        return;
    };
    let Some(job) = sam2::pending_analysis(project, scheduled) else {
        return;
    };
    let modifier_id = job.modifier_id;
    let run_id = job.run_id;
    if sam2::spawn_analysis(project, job, event_tx.clone(), &claim_waiter) {
        scheduled.insert(modifier_id, run_id);
    }
}

fn take_pending_project(
    pending: &Mutex<Option<(Arc<Project>, u64, u64)>>,
    notification_pending: &AtomicBool,
) -> Option<(Arc<Project>, u64, u64)> {
    let mut pending = pending
        .lock()
        .expect("pending video project mutex poisoned");
    let project = pending.take();
    notification_pending.store(false, Ordering::Release);
    project
}

struct PendingProjectReset {
    project: PendingProject,
    notification: Arc<AtomicBool>,
}

impl Drop for PendingProjectReset {
    fn drop(&mut self) {
        self.project
            .lock()
            .expect("pending video project mutex poisoned")
            .take();
        self.notification.store(false, Ordering::Release);
    }
}

fn retain_project_sessions(project: &Project, sessions: &mut RenderSessions) {
    let tracks = scoped_video_tracks(project);
    sessions.sources.retain(
        |sequence_path, track_id, item_id, media_track_id| {
            tracks
                .get(&ScopedTrackKey {
                    sequence_path: sequence_path.to_vec(),
                    track_id,
                })
                .is_some_and(|track| {
                    track.items.iter().any(|item| {
                        item.id == item_id
                            && (item.track_id == media_track_id
                                || item.alpha_mask_video == Some(media_track_id))
                    })
                })
        },
        |sequence_path, track_id, item_id| {
            tracks
                .get(&ScopedTrackKey {
                    sequence_path: sequence_path.to_vec(),
                    track_id,
                })
                .is_some_and(|track| track.items.iter().any(|item| item.id == item_id))
        },
        |file| {
            tracks.values().any(|track| {
                track.items.iter().any(|item| {
                    matches!(
                        item.content,
                        shrimply_project::project::VideoItemContent::LayeredImage(_)
                    ) && item.file.path() == file
                })
            })
        },
    );
    let mut retained_manim = 0;
    let mut pruned_manim = 0;
    sessions.elements.retain(|key, element| {
        let retain = match key {
            VisualElementKey::Item {
                sequence_path,
                track_id,
                item_id,
                media_track_id,
                plane,
            } => tracks
                .get(&ScopedTrackKey {
                    sequence_path: sequence_path.clone(),
                    track_id: *track_id,
                })
                .is_some_and(|track| {
                    track.items.iter().any(|item| {
                        let expected_plane = if item.track_id == *media_track_id {
                            VideoPlane::Color
                        } else if item.alpha_mask_video == Some(*media_track_id) {
                            VideoPlane::Alpha
                        } else {
                            return false;
                        };
                        item.id == *item_id && expected_plane == *plane
                    })
                }),
            VisualElementKey::Manim {
                sequence_path,
                track_id,
                item_id,
                width,
                height,
            } => tracks
                .get(&ScopedTrackKey {
                    sequence_path: sequence_path.clone(),
                    track_id: *track_id,
                })
                .is_some_and(|track| {
                    track.items.iter().any(|item| {
                        item.id == *item_id
                            && element.matches(
                                item,
                                shrimply_project::project::CanvasSize {
                                    width: *width,
                                    height: *height,
                                },
                            )
                    })
                }),
        };
        if let VisualElementKey::Manim {
            item_id,
            width,
            height,
            ..
        } = key
        {
            if retain {
                retained_manim += 1;
            } else {
                pruned_manim += 1;
                tracing::debug!(
                    item = %item_id,
                    width,
                    height,
                    "pruning stale Manim render session after project update",
                );
            }
        }
        retain
    });
    if pruned_manim > 0 {
        tracing::debug!(
            retained = retained_manim,
            pruned = pruned_manim,
            "reconciled Manim render sessions after project update",
        );
    }
    sessions
        .decoders
        .retain(tracks.iter().flat_map(|(scope, track)| {
            track
                .items
                .iter()
                .map(move |item| (scope.sequence_path.as_slice(), scope.track_id, item))
        }));
    sessions.volume_revision = sessions.volume_revision.wrapping_add(1);
}

fn scoped_video_tracks(
    project: &Project,
) -> HashMap<ScopedTrackKey, &shrimply_project::project::VisualTrack> {
    fn collect<'a>(
        project: &'a Project,
        reference: SequenceReference,
        path: &mut Vec<Uuid>,
        tracks: &mut HashMap<ScopedTrackKey, &'a shrimply_project::project::VisualTrack>,
        stack: &mut Vec<Uuid>,
    ) {
        if stack.contains(&reference.sequence_id) {
            return;
        }
        let Some(sequence) = project.folded_sequence(reference.sequence_id) else {
            return;
        };
        stack.push(reference.sequence_id);
        for track in &sequence.video_tracks {
            tracks.insert(
                ScopedTrackKey {
                    sequence_path: path.clone(),
                    track_id: track.id,
                },
                track,
            );
            for item in &track.items {
                let shrimply_project::project::VideoItemContent::FoldedSequence(reference) =
                    item.content
                else {
                    continue;
                };
                path.push(item.id);
                collect(project, reference, path, tracks, stack);
                path.pop();
            }
        }
        stack.pop();
    }

    let mut tracks = HashMap::new();
    for track in &project.video_tracks {
        tracks.insert(
            ScopedTrackKey {
                sequence_path: Vec::new(),
                track_id: track.id,
            },
            track,
        );
        for item in &track.items {
            let shrimply_project::project::VideoItemContent::FoldedSequence(reference) =
                item.content
            else {
                continue;
            };
            let mut path = vec![item.id];
            collect(project, reference, &mut path, &mut tracks, &mut Vec::new());
        }
    }
    tracks
}
#[cfg(test)]
mod cancellation_tests {
    use super::*;

    #[test]
    fn continuous_playback_coalesces_until_an_explicit_discontinuity() {
        let (sender, receiver) = mpsc::channel();
        let cancel_generation = Arc::new(AtomicU64::new(0));
        let commands = VideoCommandSender {
            sender,
            _sam2_claim_waiter: Arc::new(crate::sam2_analysis::ClaimWaiter::new(|| {})),
            next_request_id: Arc::new(AtomicU64::new(0)),
            cancel_generation: cancel_generation.clone(),
            last_render: Arc::new(Mutex::new(None)),
            latest_project_generation: Arc::new(AtomicU64::new(0)),
            pending_project: Arc::new(Mutex::new(None)),
            project_notification_pending: Arc::new(AtomicBool::new(false)),
            playback_observer: None,
        };

        commands
            .send(VideoCommand::Render {
                position: Time::ZERO,
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send first playback render");
        let first_generation = match receiver.recv().expect("receive first playback render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 1);
                cancel_generation
            }
            _ => panic!("first playback command was not a render"),
        };
        let active = DecodeControl::new(first_generation, cancel_generation.clone());

        commands
            .send(VideoCommand::Render {
                position: Time::from_fraction(1, 30),
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send adjacent playback render");
        match receiver.recv().expect("receive adjacent playback render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 2);
                assert_eq!(cancel_generation, first_generation);
            }
            _ => panic!("adjacent playback command was not a render"),
        }
        assert!(
            !active.superseded(),
            "adjacent playback canceled active work"
        );

        commands
            .send(VideoCommand::Render {
                position: Time::from_seconds(1),
                accuracy: CompositeAccuracy::CONTINUOUS_TIME_ACCURATE,
            })
            .expect("send accelerated playback render");
        match receiver
            .recv()
            .expect("receive accelerated playback render")
        {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 3);
                assert_eq!(cancel_generation, first_generation);
            }
            _ => panic!("accelerated playback command was not a render"),
        }
        assert!(
            !active.superseded(),
            "accelerated playback canceled active work"
        );

        commands
            .send(VideoCommand::Render {
                position: Time::from_seconds(2),
                accuracy: CompositeAccuracy::TIME_ACCURATE,
            })
            .expect("send discontinuous render");
        match receiver.recv().expect("receive discontinuous render") {
            WorkerCommand::Render {
                request_id,
                cancel_generation,
                ..
            } => {
                assert_eq!(request_id, 4);
                assert!(cancel_generation > first_generation);
            }
            _ => panic!("discontinuous command was not a render"),
        }
        assert!(
            active.superseded(),
            "discontinuity did not cancel active work"
        );
        assert!(
            !commands.render_generation_is_current(first_generation),
            "queued output from the old playback generation remained current"
        );
        assert!(commands.render_generation_is_current(cancel_generation.load(Ordering::Acquire)));
    }
}
