mod decode;
mod generated;
mod items;
mod layers;
pub mod math;
mod media;

pub use shrimply_project_evaluation::FrameAudioAnalysis;

use shrimply_math_core::Time;
pub use shrimply_preview_provider_skia::accuracy::CompositeAccuracy;
use shrimply_preview_provider_skia::accuracy::{FINAL_PREVIEW_DELAY, LOCAL_SCRUB_WINDOW_SECONDS};
use shrimply_project_document::project::{
    ItemAddress, Project, TrackAddress, VideoItemContent, video_source_time_at,
};
use shrimply_project_evaluation::{
    TransformExpressionCache, VisualEvaluation, resolve_bool, resolve_scalar,
};
use shrimply_render_core::{LayerKind, Nv12LayerParams, TextureAddressMode};
use skia_safe::Image;
use std::time::Instant;

pub struct Layer {
    pub parameters: Nv12LayerParams,
    pub transform: shrimply_render_core::math::Mat3,
    pub source: Source,
    pub transitions: Vec<TransitionStage>,
    pub effects: Vec<shrimply_visual_core::raster_modifiers::Modifier>,
    pub render_size: (u32, u32),
    pub output_transform: shrimply_render_core::math::Mat3,
    pub motion_blur: Option<Vec<shrimply_math_geometry::ComposedTransform2D>>,
    pub morph_scene: Option<shrimply_visual_core::vector_morph::MorphScene>,
    pub alpha_mask: Option<shrimply_visual_core::alpha_mask::ResolvedShapeAlphaMask>,
    pub video_mask: Option<VideoMask>,
    pub stabilization: Option<shrimply_visual_core::stabilization::StabilizationWarp>,
}

pub struct VideoMask {
    pub image: Image,
    pub size: (u32, u32),
    pub sampling: shrimply_render_core::VideoSampleMethod,
}

pub enum Source {
    Generated(Box<shrimply_visual_core::generated::GeneratedFrame>),
    Group(Vec<Layer>),
    Image(Image),
    Background(Box<shrimply_render_core::background_spirv::BackgroundUniforms>),
    Manim(ManimFrame),
    RasterMorph(Box<RasterMorph>),
    LayeredImage(Box<shrimply_visual_core::layered_image::Prepared>),
    Gaussian(Box<shrimply_visual_core::gaussian::Prepared>),
    Obj(Box<shrimply_visual_core::obj::Prepared>),
}

pub struct RasterMorph {
    pub key: MorphCacheKey,
    pub outgoing: Box<Layer>,
    pub incoming: Box<Layer>,
    pub progress: f32,
    pub cacheable: bool,
}

pub struct ManimFrame {
    pub item_id: uuid::Uuid,
    pub source: shrimply_manim_wgpu::SourceIdentity,
    pub prepared: std::sync::Arc<shrimply_manim_wgpu::PreparedAnimation>,
    pub frame_index: usize,
}

pub struct TransitionStage {
    pub transform: shrimply_render_core::math::Mat3,
    pub effect: Option<shrimply_visual_core::transition::RasterTransition>,
}

pub struct FramePlan {
    pub time: Time,
    pub accuracy: CompositeAccuracy,
    pub loading: bool,
    pub audio_analysis: FrameAudioAnalysis,
    pub layers: Vec<Layer>,
    pub external_layers: Vec<ExternalLayers>,
    pub width: u32,
    pub height: u32,
}

pub struct ExternalLayers {
    pub dependency: shrimply_visual_core::raster_modifiers::ExternalDependency,
    pub layers: Vec<Layer>,
}

struct PreparedDependency<'a> {
    dependency: shrimply_visual_core::raster_modifiers::ExternalDependency,
    audio: FrameAudioAnalysis,
    items: Vec<items::PreparedItem<'a>>,
}

#[derive(Default)]
struct DependencyTraversal<'a> {
    stack: Vec<ItemAddress>,
    records: Vec<PreparedDependency<'a>>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MorphCacheKey {
    sequence_path: Vec<uuid::Uuid>,
    track_id: uuid::Uuid,
    outgoing_id: uuid::Uuid,
    incoming_id: uuid::Uuid,
    width: u32,
    height: u32,
    content_revision: u64,
    cacheable: bool,
}

/// Selects capture output while retaining the full project for external dependencies.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CaptureTarget {
    Item(ItemAddress),
    Track(TrackAddress),
    ModifierInput {
        address: ItemAddress,
        snap_content: bool,
    },
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
    capture_target: Option<CaptureTarget>,
    audio_sampler: shrimply_audio_engine::streaming::FrameAudioSampler,
    audio_revision: u64,
    audio_pending: bool,
    sampled_audio: Vec<FrameAudioAnalysis>,
    morphs: std::collections::HashMap<
        MorphCacheKey,
        std::rc::Rc<shrimply_visual_core::vector_morph::PreparedVectorMorph>,
    >,
    manim: std::collections::HashMap<uuid::Uuid, shrimply_manim_wgpu::Source>,
    manim_updates: Vec<shrimply_manim_state::Update>,
    manim_loading: bool,
    manim_pending: bool,
    blender: std::collections::HashMap<uuid::Uuid, shrimply_visual_core::blender::Source>,
    blender_images: std::collections::HashMap<
        uuid::Uuid,
        (std::sync::Arc<shrimply_visual_core::blender::Frame>, Image),
    >,
    blender_loading_image: Option<(shrimply_project_document::project::CanvasSize, Image)>,
    blender_loading: bool,
    gaussians: std::collections::HashMap<uuid::Uuid, shrimply_visual_core::gaussian::Source>,
    objs: std::collections::HashMap<uuid::Uuid, shrimply_visual_core::obj::State>,
    stabilization_pending: bool,
}

impl Scene {
    pub fn set_decoder_limit(&mut self, maximum: usize) {
        self.media.set_decoder_limit(maximum);
    }

    pub fn take_manim_updates(&mut self) -> Vec<shrimply_manim_state::Update> {
        std::mem::take(&mut self.manim_updates)
    }

    pub fn set_exclusion(&mut self, excluded_item_id: Option<uuid::Uuid>) {
        if self.excluded_item_id != excluded_item_id {
            self.excluded_item_id = excluded_item_id;
            self.prepared = None;
        }
    }

    pub fn set_capture_target(&mut self, target: Option<CaptureTarget>) -> bool {
        if self.capture_target == target {
            return false;
        }
        self.capture_target = target;
        self.prepared = None;
        true
    }

    pub fn needs_update(&self) -> bool {
        self.media.needs_update()
            || self.audio_pending
            || self.manim_loading
            || self.manim_pending
            || self.blender_loading
            || self.stabilization_pending
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
        self.blender.clear();
        self.blender_images.clear();
        self.blender_loading_image = None;
        self.blender_loading = false;
        self.gaussians.clear();
        self.objs.clear();
        self.stabilization_pending = false;
    }

    pub fn prepare(&mut self, project: &Project, time: Time) -> Result<Option<FramePlan>, String> {
        self.manim.retain(|item_id, _| {
            project
                .video_item_by_id(*item_id)
                .is_some_and(|item| matches!(item.content, VideoItemContent::Manim(_)))
        });
        self.blender.retain(|item_id, _| {
            project
                .video_item_by_id(*item_id)
                .is_some_and(|item| matches!(item.content, VideoItemContent::Blender(_)))
        });
        self.blender_images
            .retain(|item_id, _| self.blender.contains_key(item_id));
        self.gaussians.retain(|item_id, source| {
            project
                .video_item_by_id(*item_id)
                .is_some_and(|item| source.matches(item))
        });
        self.objs.retain(|item_id, state| {
            project
                .video_item_by_id(*item_id)
                .is_some_and(|item| state.matches(item))
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
        let capture_target = self.capture_target.clone();
        let items = self.items(
            project,
            &project.video_tracks,
            &audio,
            &items::Scope {
                time,
                ..Default::default()
            },
            capture_target.as_ref().map(|target| match target {
                CaptureTarget::Item(address) => items::Target::Item {
                    address,
                    scope_positions: None,
                    modifier_input: None,
                },
                CaptureTarget::ModifierInput {
                    address,
                    snap_content,
                } => items::Target::Item {
                    address,
                    scope_positions: None,
                    modifier_input: Some(*snap_content),
                },
                CaptureTarget::Track(address) => items::Target::Track(address),
            }),
            &mut requests,
        )?;
        let mut traversal = DependencyTraversal::default();
        for (dependency, dependency_audio) in external_dependencies(project, &items, &audio)? {
            self.collect_external_dependency(
                project,
                time,
                dependency,
                dependency_audio,
                &mut requests,
                &mut traversal,
            )?;
        }
        let dependency_records = traversal.records;
        if !self.media.request(requests)? {
            return Ok(None);
        }
        let key = (time, self.media.revision(), accuracy);
        if self.prepared == Some(key)
            && !self.audio_pending
            && !self.manim_loading
            && !self.blender_loading
        {
            return Ok(None);
        }
        self.manim_loading = false;
        self.manim_pending = false;
        self.blender_loading = false;
        self.stabilization_pending = false;
        let mut external_layers = Vec::with_capacity(dependency_records.len());
        for dependency in dependency_records {
            external_layers.push(ExternalLayers {
                dependency: dependency.dependency,
                layers: self.layers(project, &dependency.audio, dependency.items)?,
            });
        }
        let layers = self.layers(project, &audio, items)?;
        if self.manim_pending || self.stabilization_pending {
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
            loading: self.manim_loading || self.blender_loading,
            audio_analysis: audio,
            layers,
            external_layers,
            width: project.canvas_size.width,
            height: project.canvas_size.height,
        }))
    }
}

fn external_dependencies(
    project: &Project,
    items: &[items::PreparedItem<'_>],
    audio: &FrameAudioAnalysis,
) -> Result<
    Vec<(
        shrimply_visual_core::raster_modifiers::ExternalDependency,
        FrameAudioAnalysis,
    )>,
    String,
> {
    let mut dependencies = Vec::new();
    for prepared in items {
        let item_audio = prepared.audio.as_ref().unwrap_or(audio);
        if let Some(children) = &prepared.children {
            for dependency in external_dependencies(project, children, item_audio)? {
                if !dependencies
                    .iter()
                    .any(|(current, _)| current == &dependency.0)
                {
                    dependencies.push(dependency);
                }
            }
        }
        for dependency in shrimply_visual_core::raster_modifiers::external_dependencies(
            shrimply_visual_core::raster_modifiers::ChainRequest {
                project,
                address: &prepared.address,
                item: &prepared.item,
                position: prepared.time,
                scope_positions: &prepared.scope_positions,
                require_complete_assets: false,
            },
        )? {
            if !dependencies
                .iter()
                .any(|(current, _)| current == &dependency)
            {
                dependencies.push((dependency, item_audio.clone()));
            }
        }
    }
    Ok(dependencies)
}

impl Scene {
    fn collect_external_dependency<'a>(
        &mut self,
        project: &'a Project,
        root_time: Time,
        dependency: shrimply_visual_core::raster_modifiers::ExternalDependency,
        audio: FrameAudioAnalysis,
        requests: &mut Vec<media::Request>,
        traversal: &mut DependencyTraversal<'a>,
    ) -> Result<(), String> {
        if traversal
            .records
            .iter()
            .any(|record| record.dependency == dependency)
        {
            return Ok(());
        }
        if traversal.stack.contains(&dependency.address) {
            return Err("cyclic mask reference".to_string());
        }
        traversal.stack.push(dependency.address.clone());
        let dependency_items = self.items(
            project,
            &project.video_tracks,
            &audio,
            &items::Scope {
                time: root_time,
                ..Default::default()
            },
            Some(items::Target::Item {
                address: &dependency.address,
                scope_positions: Some(&dependency.scope_positions),
                modifier_input: None,
            }),
            requests,
        )?;
        for (child, child_audio) in external_dependencies(project, &dependency_items, &audio)? {
            self.collect_external_dependency(
                project,
                root_time,
                child,
                child_audio,
                requests,
                traversal,
            )?;
        }
        traversal.stack.pop();
        traversal.records.push(PreparedDependency {
            dependency,
            audio,
            items: dependency_items,
        });
        Ok(())
    }
}
