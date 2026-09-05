mod decode;
mod generated;
mod items;
mod layers;
pub mod math;
mod media;

pub use shrimply_evaluation::FrameAudioAnalysis;

use shrimply_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_bool, resolve_scalar,
};
use shrimply_math_core::Time;
pub use shrimply_preview_core::accuracy::CompositeAccuracy;
use shrimply_preview_core::accuracy::{FINAL_PREVIEW_DELAY, LOCAL_SCRUB_WINDOW_SECONDS};
use shrimply_project::project::{Project, VideoItemContent, video_source_time_at};
use shrimply_render_core::{LayerKind, Nv12LayerParams, TextureAddressMode};
use skia_safe::Image;
use std::time::Instant;

pub struct Layer {
    pub parameters: Nv12LayerParams,
    pub transform: shrimply_render_core::math::Mat3,
    pub source: Source,
    pub transitions: Vec<TransitionStage>,
    pub effects: Vec<shrimply_video_core::raster_modifiers::Modifier>,
    pub render_size: (u32, u32),
    pub output_transform: shrimply_render_core::math::Mat3,
    pub motion_blur: Option<Vec<shrimply_math_geometry::ComposedTransform2D>>,
    pub morph_scene: Option<shrimply_video_core::vector_morph::MorphScene>,
    pub alpha_mask: Option<shrimply_video_core::alpha_mask::ResolvedShapeAlphaMask>,
    pub video_mask: Option<VideoMask>,
}

pub struct VideoMask {
    pub image: Image,
    pub size: (u32, u32),
    pub sampling: shrimply_render_core::VideoSampleMethod,
}

pub enum Source {
    Generated(Box<shrimply_video_core::generated::GeneratedFrame>),
    Group(Vec<Layer>),
    Image(Image),
    Background(Box<shrimply_render_core::background_spirv::BackgroundUniforms>),
    Manim(ManimFrame),
}

pub struct ManimFrame {
    pub item_id: uuid::Uuid,
    pub prepared: std::sync::Arc<shrimply_manim_wgpu::PreparedAnimation>,
    pub frame_index: usize,
}

pub struct TransitionStage {
    pub transform: shrimply_render_core::math::Mat3,
    pub effect: Option<shrimply_render_core::effects::PixelEffect>,
}

pub struct FramePlan {
    pub time: Time,
    pub accuracy: CompositeAccuracy,
    pub loading: bool,
    pub audio_analysis: FrameAudioAnalysis,
    pub layers: Vec<Layer>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MorphCacheKey {
    sequence_path: Vec<uuid::Uuid>,
    track_id: uuid::Uuid,
    outgoing_id: uuid::Uuid,
    incoming_id: uuid::Uuid,
    width: u32,
    height: u32,
}

/// Shared media scheduling and evaluated layer inputs. Pixel rendering belongs to
/// the shared Slang kernels dispatched by the selected GPU backend.
#[derive(Default)]
pub struct Scene {
    paint_caches: std::collections::HashMap<
        uuid::Uuid,
        std::rc::Rc<std::cell::RefCell<shrimply_paint_skia::PaintCache>>,
    >,
    media: media::Media,
    expressions: TransformExpressionCache,
    previous_time: Option<Time>,
    playing: bool,
    scrubbing: bool,
    moved_at: Option<Instant>,
    accuracy: CompositeAccuracy,
    requested_accuracy: CompositeAccuracy,
    prepared: Option<(Time, u64, CompositeAccuracy)>,
    excluded_item_id: Option<uuid::Uuid>,
    audio_sampler: shrimply_audio::streaming::FrameAudioSampler,
    audio_revision: u64,
    audio_pending: bool,
    sampled_audio: Vec<FrameAudioAnalysis>,
    morphs: std::collections::HashMap<
        MorphCacheKey,
        std::rc::Rc<shrimply_video_core::vector_morph::PreparedVectorMorph>,
    >,
    manim: std::collections::HashMap<uuid::Uuid, shrimply_manim_wgpu::Source>,
    manim_updates: Vec<shrimply_state::manim_status::Update>,
    manim_loading: bool,
    manim_pending: bool,
}

impl Scene {
    pub fn take_manim_updates(&mut self) -> Vec<shrimply_state::manim_status::Update> {
        std::mem::take(&mut self.manim_updates)
    }

    pub fn set_exclusion(&mut self, excluded_item_id: Option<uuid::Uuid>) {
        if self.excluded_item_id != excluded_item_id {
            self.excluded_item_id = excluded_item_id;
            self.prepared = None;
        }
    }

    pub fn needs_update(&self) -> bool {
        self.media.needs_update()
            || self.audio_pending
            || self.manim_pending
            || self.scrubbing && self.requested_accuracy != CompositeAccuracy::FULLY_ACCURATE
    }

    pub fn set_interaction(&mut self, playing: bool, scrubbing: bool) {
        self.playing = playing;
        self.scrubbing = scrubbing;
    }

    pub fn invalidate(&mut self) {
        self.media.invalidate();
        self.paint_caches.clear();
        self.expressions = TransformExpressionCache::default();
        self.previous_time = None;
        self.moved_at = None;
        self.prepared = None;
        self.audio_revision = self.audio_revision.wrapping_add(1);
        self.audio_pending = false;
        self.sampled_audio.clear();
        self.morphs.clear();
        self.manim_loading = false;
        self.manim_pending = false;
    }

    pub fn prepare(&mut self, project: &Project, time: Time) -> Result<Option<FramePlan>, String> {
        self.manim.retain(|item_id, _| {
            project
                .video_item_by_id(*item_id)
                .is_some_and(|item| matches!(item.content, VideoItemContent::Manim(_)))
        });
        let now = Instant::now();
        if self.previous_time != Some(time) {
            let local = self.previous_time.is_some_and(|previous| {
                time.abs_diff(previous) <= Time::from_seconds(LOCAL_SCRUB_WINDOW_SECONDS)
            });
            self.accuracy = if local {
                CompositeAccuracy::LOCAL_TIME_ACCURATE
            } else {
                CompositeAccuracy::BEST_EFFORT
            };
            self.moved_at = Some(now);
        }
        let settled = self
            .moved_at
            .is_none_or(|moved| now.duration_since(moved) >= FINAL_PREVIEW_DELAY);
        let accuracy = if self.playing {
            CompositeAccuracy::CONTINUOUS_TIME_ACCURATE
        } else if !self.scrubbing || settled {
            CompositeAccuracy::FULLY_ACCURATE
        } else {
            self.accuracy
        };
        self.previous_time = Some(time);
        self.requested_accuracy = accuracy;
        let audio = self
            .audio_sampler
            .sample(project, time, self.audio_revision);
        self.sampled_audio.clear();
        let mut requests = Vec::new();
        let items = self.items(
            project,
            &project.video_tracks,
            &audio,
            &items::Scope {
                time,
                ..Default::default()
            },
            &mut requests,
        )?;
        if !self.media.request(requests)? {
            return Ok(None);
        }
        let key = (time, self.media.revision(), accuracy);
        if self.prepared == Some(key) && !self.audio_pending && !self.manim_loading {
            return Ok(None);
        }
        self.manim_loading = false;
        self.manim_pending = false;
        let layers = self.layers(project, &audio, items)?;
        if self.manim_pending {
            return Ok(None);
        }
        let failures = std::iter::once(&audio)
            .chain(&self.sampled_audio)
            .flat_map(FrameAudioAnalysis::failures)
            .collect::<Vec<_>>();
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        self.audio_pending = std::iter::once(&audio)
            .chain(&self.sampled_audio)
            .any(FrameAudioAnalysis::pending);
        if self.audio_pending && accuracy.content_accurate() {
            return Ok(None);
        }
        self.prepared = Some(key);
        Ok(Some(FramePlan {
            time,
            accuracy,
            loading: self.manim_loading,
            audio_analysis: audio,
            layers,
            width: project.canvas_size.width,
            height: project.canvas_size.height,
        }))
    }
}
