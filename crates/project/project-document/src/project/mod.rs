use hashbrown::HashSet;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use glam::{UVec2, Vec2};
use serde::{Deserialize, Serialize};
pub use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_core::{Fraction, GenericFraction};
pub use shrimply_math_geometry::{ResolvedTransform2D as ResolvedTransform, Transform2D};
pub use shrimply_math_interpolation::Interpolation;
pub use shrimply_property_model::{
    Color, FRACTION_ZERO, LayerBlendMode, SkiaDrawingStrategy, TextureAddressMode, Time,
    VideoSampleMethod, VisualEdges, deserialize_fraction, fraction_as_f64, fraction_as_label,
    fraction_denominator, fraction_from_integer, fraction_new, fraction_numerator,
    serialize_fraction,
};
use uuid::Uuid;

use shrimply_audio_modifiers::{AudioModifier, GainModifier};
use shrimply_property_model::timeline_value::*;
use shrimply_scene_3d::{AnimatedVec3, ObjScene};
use shrimply_visual_modifiers::{ModifierEffect, ModifierModel};

mod audio_generator;
mod conversion;
mod generated;
#[cfg(feature = "editor")]
mod history;
mod item_address;
mod lifecycle;
mod ownership;
mod paint;
mod preview;
mod timing;
pub use crate::caption::{
    CaptionEdgeStyle, CaptionFont, CaptionItem, CaptionWritingDirection, HorizontalAlign,
    VerticalAlign,
};
pub use audio_generator::{
    AudioGenerator, AudioWaveform, DEFAULT_AUDIO_GENERATOR_FREQUENCY_HZ,
    DEFAULT_AUDIO_GENERATOR_PULSE_WIDTH,
};
pub use generated::VisualSource as VideoItemContent;
pub use generated::{
    BlenderItem, BlenderPreviewDownsample, BlenderRenderMethod, DEFAULT_TEXT_FONT_FAMILY,
    FontFamily, FontVariation, LayerVisibility, LayeredImageItem, ManimItem, ManimParameter,
    ManimParameterControl, ManimParameterValue, PdfItem, ShapeItem, ShapeKind,
    ShapeRoundingStrategy, TextDirection, TextFontStyle, TextHorizontalAlign, TextItem,
    VisualSource,
};
#[cfg(feature = "editor")]
pub use history::{
    CommitStatus, PreparedProject, ProjectPreparation, activate_project, can_redo, can_undo,
    commit_coalesced_edit, commit_edit, commit_edit_checked, connect_commit_status,
    create_new_project_file, create_project_file, finish_coalesced_edit, poll_commit_status,
    prepare_project, prepare_project_with_frame_grid_repair, redo, save, save_as, save_view_state,
    serialize_project_json, shutdown_history, undo,
};
pub use item_address::{
    ItemAddress, ItemKind, ItemMut, ItemRef, ProjectItem, SequenceScopeId, TrackAddress, TrackMut,
    TrackRef,
};
pub use lifecycle::*;
pub use ownership::{
    ProjectLoadError, ProjectLockError, ProjectLockOwner, acquire_project_lock,
    clear_project_file_locks, normalized_project_path, project_lock_owner, release_project_lock,
    terminate_project_process,
};
pub use preview::{
    COMPOSITING_ALPHA_MASK_PREVIEW_FACET, ITEM_PREVIEW_FACET, MODIFIER_ALPHA_MASK_PREVIEW_FACET,
    SHAPE_APPEARANCE_PREVIEW_FACET, SHAPE_CONTENT_PREVIEW_FACET, TEXT_APPEARANCE_PREVIEW_FACET,
    TRACKED_CAMERA_PREVIEW, TrackedCameraPreview,
};
pub use shrimply_background::{
    Background, BackgroundGenerator, BackgroundKind, CenteredLines, Checkerboard, ColorGradient,
    Curve, GradientMode, Grid, GridLineStyle, NoiseColorMode, NoiseDistribution, PerlinMode,
    PerlinNoise, Rainbow, RainbowBands, RainbowFill, SolidColor, Voronoi, VoronoiFill,
    VoronoiMetric, WhiteNoise,
};
pub use shrimply_paint_model::{
    DEFAULT_FILL_CLOSURE_TOLERANCE, DEFAULT_STROKE_WIDTH, DEFAULT_STROKE_WIDTH_SCALE, PaintDrawing,
    PaintDrawingKeyframe, PaintFill, PaintFillOptions, PaintItem, PaintPaletteEntry, PaintPoint,
    PaintStroke, PaintStrokeEndOptions, PaintStrokeOptions, PaintTaper, PaintTextureOptions,
    PaintTransform, ResolvedPaintFillOptions, ResolvedPaintStrokeEndOptions,
    ResolvedPaintStrokeOptions, ResolvedPaintTextureOptions,
};
pub use shrimply_path_core::{active_project_path, project_directory, set_active_project_path};
pub use shrimply_project_types::{
    AudioClipTransitionCurve, COMMON_FRAME_RATES, CanvasSize, DEFAULT_CANVAS_SIZE,
    DEFAULT_PROJECT_FPS, FrameRate, MAX_CANVAS_DIMENSION, MIN_CANVAS_DIMENSION, PROJECT_PRESETS,
    ProjectPreset, TransitionSide, clamp_item_end_to_next_start,
};
pub use timing::*;

pub const PROJECT_FORMAT_VERSION: u32 = 32;
pub const DEFAULT_MOTION_BLUR_SHUTTER_ANGLE_DEGREES: u32 = 180;
pub const DEFAULT_MOTION_BLUR_SHUTTER_PHASE_DEGREES: i32 = -90;
pub const DEFAULT_MOTION_BLUR_SAMPLES: u32 = 8;
pub const MIN_MOTION_BLUR_SHUTTER_ANGLE_DEGREES: u32 = 1;
pub const MAX_MOTION_BLUR_SHUTTER_ANGLE_DEGREES: u32 = 720;
pub const MIN_MOTION_BLUR_SHUTTER_PHASE_DEGREES: i32 = -360;
pub const MAX_MOTION_BLUR_SHUTTER_PHASE_DEGREES: i32 = 360;
pub const MIN_MOTION_BLUR_SAMPLES: u32 = 2;
pub const MAX_MOTION_BLUR_SAMPLES: u32 = 64;
pub const MAX_VISUAL_CLIP_TRANSITION_SOFTNESS: f32 = 0.5;
pub const MAX_VISUAL_CLIP_TRANSITION_CLOCK_SOFTNESS: f32 = 0.25;
pub const MAX_VISUAL_CLIP_TRANSITION_DISSOLVE_GRAIN_SIZE: u32 = 64;
pub const MAX_VISUAL_CLIP_TRANSITION_ZOOM_SCALE: f32 = 2.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {
    #[serde(default = "current_project_format_version")]
    pub format_version: u32,
    #[serde(default = "default_project_name")]
    pub name: String,
    #[serde(
        default = "default_project_fps",
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub fps: Fraction,
    #[serde(default = "default_canvas_size")]
    pub canvas_size: CanvasSize,
    #[serde(default, alias = "subtitle_tracks")]
    pub caption_tracks: Vec<CaptionTrack>,
    #[serde(default, rename = "visual_tracks", alias = "video_tracks")]
    pub video_tracks: Vec<VisualTrack>,
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrack>,
    #[serde(default)]
    pub folded_sequences: Vec<FoldedSequence>,
    #[serde(default)]
    pub expanded_sequence_paths: Vec<Vec<Uuid>>,
    #[serde(default)]
    pub cursor_position: Option<Time>,
    #[serde(default)]
    pub timeline_zoom: Option<Time>,
    #[serde(default)]
    pub preview_guides: Box<PreviewGuides>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PreviewGuides {
    #[serde(default)]
    pub vertical: Vec<f32>,
    #[serde(default)]
    pub horizontal: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FoldedSequence {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default)]
    pub video_tracks: Vec<VisualTrack>,
    #[serde(default)]
    pub audio_tracks: Vec<AudioTrack>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SequenceReference {
    pub sequence_id: Uuid,
    #[serde(default = "Uuid::new_v4")]
    pub instance_id: Uuid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptionTrack {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub language: Option<String>,
    pub items: Vec<CaptionItem>,
}

pub fn caption_languages() -> &'static BTreeSet<String> {
    static LANGUAGES: OnceLock<BTreeSet<String>> = OnceLock::new();
    LANGUAGES.get_or_init(|| {
        let languages =
            icu_locale::fallback::provider::Baked::SINGLETON_LOCALE_LIKELY_SUBTAGS_LANGUAGE_V1;
        let subtags = icu_locale::provider::Baked::SINGLETON_LOCALE_LIKELY_SUBTAGS_SCRIPT_REGION_V1;
        let extended = icu_locale::provider::Baked::SINGLETON_LOCALE_LIKELY_SUBTAGS_EXTENDED_V1;
        let mut options = BTreeSet::new();

        for (language, (_, region)) in languages
            .language
            .iter_copied()
            .chain(extended.language.iter_copied())
        {
            let Ok(language) = language.try_into_tinystr() else {
                continue;
            };
            options.insert(format!("{}_{}", language.as_str(), region.as_str()));
        }
        for (region, (language, _)) in subtags
            .region
            .iter_copied()
            .chain(extended.region.iter_copied())
        {
            let Ok(region) = region.try_into_tinystr() else {
                continue;
            };
            options.insert(format!("{}_{}", language.as_str(), region.as_str()));
        }
        for ((language, region), _) in languages
            .language_region
            .iter_copied()
            .chain(extended.language_region.iter_copied())
        {
            let (Ok(language), Ok(region)) =
                (language.try_into_tinystr(), region.try_into_tinystr())
            else {
                continue;
            };
            options.insert(format!("{}_{}", language.as_str(), region.as_str()));
        }
        options
    })
}

pub fn supported_caption_language(language: &Option<String>) -> Option<String> {
    language
        .as_ref()
        .filter(|language| caption_languages().contains(language.as_str()))
        .cloned()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualTrack {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub items: Vec<VisualItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioTrack {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub gain_db: f32,
    pub items: Vec<AudioItem>,
}

pub const AUDIO_TRACK_GAIN_MIN_DB: f32 = -60.0;
pub const AUDIO_TRACK_GAIN_MAX_DB: f32 = 36.0;

fn default_audio_track_gain_db() -> f32 {
    0.0
}

impl Default for CaptionTrack {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            enabled: true,
            language: None,
            items: Vec::new(),
        }
    }
}

impl Default for VisualTrack {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            enabled: true,
            items: Vec::new(),
        }
    }
}

impl Default for AudioTrack {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            enabled: true,
            gain_db: default_audio_track_gain_db(),
            items: Vec::new(),
        }
    }
}

impl FoldedSequence {
    pub fn duration(&self) -> Time {
        self.video_tracks
            .iter()
            .flat_map(|track| track.items.iter().map(|item| item.end))
            .chain(
                self.audio_tracks
                    .iter()
                    .flat_map(|track| track.items.iter().map(|item| item.end)),
            )
            .max()
            .unwrap_or(Time::ZERO)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct SvgColorOverride {
    pub kind: SvgPaintKind,
    pub original: Color<u8>,
    pub replacement: Color<u8>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SvgPaintKind {
    Fill,
    Stroke,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Serialize,
    Deserialize,
    Eq,
    PartialEq,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum RepeatStrategy {
    Repeat,
    #[strum(to_string = "Ping Pong")]
    PingPong,
    #[default]
    Hold,
    Empty,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoStabilizationMethod {
    Off,
    #[default]
    L1,
    MeshFlow,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshFlowAdaptiveWeights {
    #[default]
    Original,
    Flipped,
    ConstantHigh,
    ConstantLow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualItem {
    #[serde(default = "uuid::Uuid::new_v4")]
    pub id: Uuid,
    pub start: Time,
    pub end: Time,
    pub time_offset: Time,
    pub source_duration: Time,
    #[serde(
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub playback_speed: Fraction,
    #[serde(
        default = "native_playback_fps",
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub playback_fps: Fraction,
    pub repeat_strategy: RepeatStrategy,
    #[serde(default)]
    pub stabilize_video: bool,
    #[serde(default)]
    pub stabilization_method: VideoStabilizationMethod,
    #[serde(default = "default_video_stabilization_crop_ratio")]
    pub stabilization_crop_ratio: f32,
    #[serde(default = "default_video_stabilization_first_derivative_weight")]
    pub stabilization_first_derivative_weight: f32,
    #[serde(default = "default_video_stabilization_second_derivative_weight")]
    pub stabilization_second_derivative_weight: f32,
    #[serde(default = "default_video_stabilization_third_derivative_weight")]
    pub stabilization_third_derivative_weight: f32,
    #[serde(default = "default_mesh_flow_rows")]
    pub mesh_flow_rows: u32,
    #[serde(default = "default_mesh_flow_columns")]
    pub mesh_flow_columns: u32,
    #[serde(default = "default_mesh_flow_smoothing_radius")]
    pub mesh_flow_smoothing_radius: u32,
    #[serde(default = "default_mesh_flow_iterations")]
    pub mesh_flow_iterations: u32,
    #[serde(default)]
    pub mesh_flow_adaptive_weights: MeshFlowAdaptiveWeights,
    #[serde(default)]
    pub animation_time_offset: Time,
    #[serde(default)]
    pub motion_blur: VisualMotionBlur,
    pub transform: Transform,
    #[serde(default)]
    pub modifiers: Vec<VisualModifier>,
    #[serde(default)]
    pub sample_method: TimelineValue<VideoSampleMethod>,
    #[serde(default)]
    pub skia_drawing_strategy: SkiaDrawingStrategy,
    #[serde(default)]
    pub compositing: VisualCompositing,
    #[serde(default)]
    pub visibility: TimelineValue<TimelineBool>,
    #[serde(default)]
    pub alpha_mask_video: Option<u32>,
    #[serde(default)]
    pub transitions: VisualTransitions,
    #[serde(default)]
    pub svg_color_overrides: Vec<SvgColorOverride>,
    #[serde(default)]
    pub source_width: u32,
    #[serde(default)]
    pub source_height: u32,
    #[serde(default)]
    pub default_transform: Option<Transform>,
    #[serde(default)]
    pub group_id: Option<u64>,
    #[serde(default)]
    pub render_canvas_size: Option<CanvasSize>,
    #[serde(default)]
    pub content: VisualSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_generation: Option<Box<shrimply_video_generation::VideoGenerationSettings>>,
    pub track_id: u32,
    pub file: Asset,
}

pub type VideoTrack = VisualTrack;
pub type VideoItem = VisualItem;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct VisualMotionBlur {
    pub enabled: bool,
    pub shutter_angle_degrees: u32,
    pub shutter_phase_degrees: i32,
    pub samples: u32,
}

impl Default for VisualMotionBlur {
    fn default() -> Self {
        Self {
            enabled: false,
            shutter_angle_degrees: DEFAULT_MOTION_BLUR_SHUTTER_ANGLE_DEGREES,
            shutter_phase_degrees: DEFAULT_MOTION_BLUR_SHUTTER_PHASE_DEGREES,
            samples: DEFAULT_MOTION_BLUR_SAMPLES,
        }
    }
}

pub fn default_video_stabilization_crop_ratio() -> f32 {
    0.7
}

pub fn default_video_stabilization_first_derivative_weight() -> f32 {
    10.0
}

pub fn default_video_stabilization_second_derivative_weight() -> f32 {
    1.0
}

pub fn default_video_stabilization_third_derivative_weight() -> f32 {
    100.0
}

pub fn default_mesh_flow_rows() -> u32 {
    16
}

pub fn default_mesh_flow_columns() -> u32 {
    16
}

pub fn default_mesh_flow_smoothing_radius() -> u32 {
    10
}

pub fn default_mesh_flow_iterations() -> u32 {
    100
}

impl VisualItem {
    pub fn stabilization_method(&self) -> VideoStabilizationMethod {
        if self.stabilize_video {
            self.stabilization_method
        } else {
            VideoStabilizationMethod::Off
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualModifier {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub alpha_mask: Option<VisualAlphaMask>,
    pub effect: ModifierEffect,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualCompositing {
    pub opacity: TimelineValue<f32>,
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub blend_mode: TimelineValue<LayerBlendMode>,
    #[serde(default)]
    pub alpha_mask: Option<VisualAlphaMask>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMaskShape {
    #[default]
    Rectangle,
    Ellipse,
    Polygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualAlphaMaskTarget {
    Compositing,
    Modifier(Uuid),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualAlphaMask {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub shape: AlphaMaskShape,
    pub center: TimelineValue<glam::Vec2>,
    pub size: TimelineValue<glam::Vec2>,
    pub rotation_degrees: TimelineValue<f32>,
    pub feather: TimelineValue<f32>,
    #[serde(default = "default_alpha_mask_rounding")]
    pub rounding: TimelineValue<f32>,
    #[serde(default = "default_alpha_mask_vertices")]
    pub vertices: Vec<glam::Vec2>,
    #[serde(default)]
    pub invert: bool,
}

fn default_alpha_mask_rounding() -> TimelineValue<f32> {
    TimelineValue::new_const(0.0)
}

fn default_alpha_mask_vertices() -> Vec<glam::Vec2> {
    vec![
        glam::Vec2::new(-0.5, -0.5),
        glam::Vec2::new(0.5, -0.5),
        glam::Vec2::new(0.5, 0.5),
        glam::Vec2::new(-0.5, 0.5),
    ]
}

impl Default for VisualAlphaMask {
    fn default() -> Self {
        Self::new(
            AlphaMaskShape::Rectangle,
            glam::Vec2::splat(0.5),
            glam::Vec2::ONE,
        )
    }
}

impl VisualAlphaMask {
    pub fn new(shape: AlphaMaskShape, center: glam::Vec2, size: glam::Vec2) -> Self {
        Self {
            enabled: true,
            shape,
            center: TimelineValue::new_const(center),
            size: TimelineValue::new_const(size.max(glam::Vec2::ZERO)),
            rotation_degrees: TimelineValue::new_const(0.0),
            feather: TimelineValue::new_const(0.0),
            rounding: default_alpha_mask_rounding(),
            vertices: default_alpha_mask_vertices(),
            invert: false,
        }
    }

    pub fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [&self.rotation_degrees, &self.feather, &self.rounding]
            .into_iter()
            .find(|value| value.id == id)
    }

    pub fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.rotation_degrees,
            &mut self.feather,
            &mut self.rounding,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    pub fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        [&self.center, &self.size]
            .into_iter()
            .find(|value| value.id == id)
    }

    pub fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        [&mut self.center, &mut self.size]
            .into_iter()
            .find(|value| value.id == id)
    }
}

impl Default for VisualCompositing {
    fn default() -> Self {
        Self {
            opacity: TimelineValue::<f32>::new_const(1.0),
            blend_mode: TimelineValue::new_const(LayerBlendMode::Normal),
            alpha_mask: None,
        }
    }
}

impl VisualModifier {
    pub fn new(effect: ModifierEffect) -> Self {
        Self {
            id: Uuid::new_v4(),
            enabled: true,
            alpha_mask: None,
            effect,
        }
    }

    pub fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        self.alpha_mask
            .as_ref()
            .and_then(|mask| mask.number(id))
            .or_else(|| self.effect.number(id))
    }

    pub fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        if self
            .alpha_mask
            .as_ref()
            .is_some_and(|mask| mask.number(id).is_some())
        {
            return self.alpha_mask.as_mut()?.number_mut(id);
        }
        self.effect.number_mut(id)
    }

    pub fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        self.alpha_mask
            .as_ref()
            .and_then(|mask| mask.number2(id))
            .or_else(|| self.effect.number2(id))
    }

    pub fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        if self
            .alpha_mask
            .as_ref()
            .is_some_and(|mask| mask.number2(id).is_some())
        {
            return self.alpha_mask.as_mut()?.number2_mut(id);
        }
        self.effect.number2_mut(id)
    }
}

impl VisualItem {
    pub fn alpha_mask(&self, target: VisualAlphaMaskTarget) -> Option<&VisualAlphaMask> {
        match target {
            VisualAlphaMaskTarget::Compositing => self.compositing.alpha_mask.as_ref(),
            VisualAlphaMaskTarget::Modifier(id) => self
                .modifiers
                .iter()
                .find(|modifier| modifier.id == id)?
                .alpha_mask
                .as_ref(),
        }
    }

    pub fn alpha_mask_mut(
        &mut self,
        target: VisualAlphaMaskTarget,
    ) -> Option<&mut VisualAlphaMask> {
        match target {
            VisualAlphaMaskTarget::Compositing => self.compositing.alpha_mask.as_mut(),
            VisualAlphaMaskTarget::Modifier(id) => self
                .modifiers
                .iter_mut()
                .find(|modifier| modifier.id == id)?
                .alpha_mask
                .as_mut(),
        }
    }

    pub fn set_alpha_mask(
        &mut self,
        target: VisualAlphaMaskTarget,
        mask: Option<VisualAlphaMask>,
    ) -> bool {
        let slot = match target {
            VisualAlphaMaskTarget::Compositing => &mut self.compositing.alpha_mask,
            VisualAlphaMaskTarget::Modifier(id) => {
                let Some(modifier) = self.modifiers.iter_mut().find(|modifier| modifier.id == id)
                else {
                    return false;
                };
                &mut modifier.alpha_mask
            }
        };
        *slot = mask;
        true
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AudioSpeedMethod {
    Naive,
    #[default]
    PreservePitch,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AudioSource {
    #[default]
    Media,
    FoldedSequence(SequenceReference),
    Tts(Box<shrimply_tts::TtsSettings>),
    Generator(Box<AudioGenerator>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioItem {
    #[serde(default = "uuid::Uuid::new_v4")]
    pub id: Uuid,
    pub start: Time,
    pub end: Time,
    pub time_offset: Time,
    pub source_duration: Time,
    #[serde(
        deserialize_with = "deserialize_fraction",
        serialize_with = "serialize_fraction"
    )]
    pub playback_speed: Fraction,
    pub repeat_strategy: RepeatStrategy,
    pub speed_method: AudioSpeedMethod,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub gain: Box<GainModifier>,
    #[serde(default)]
    pub beat_detection: bool,
    #[serde(default)]
    pub group_id: Option<u64>,
    #[serde(default)]
    pub source: AudioSource,
    pub track_id: u32,
    #[serde(default)]
    pub modifiers: Vec<AudioModifier>,
    #[serde(default)]
    pub transitions: AudioTransitions,
    pub file: Asset,
}

impl AudioItem {
    pub fn uses_file_asset(&self) -> bool {
        matches!(self.source, AudioSource::Media | AudioSource::Tts(_))
    }

    pub fn builder(start: Time, end: Time) -> AudioItemBuilder {
        AudioItemBuilder {
            item: Self {
                id: Uuid::new_v4(),
                start,
                end,
                time_offset: Time::ZERO,
                source_duration: Time::ZERO,
                playback_speed: default_playback_speed(),
                repeat_strategy: RepeatStrategy::default(),
                speed_method: AudioSpeedMethod::default(),
                enabled: true,
                gain: Default::default(),
                beat_detection: false,
                group_id: None,
                source: AudioSource::default(),
                track_id: 0,
                modifiers: Vec::new(),
                transitions: Default::default(),
                file: Default::default(),
            },
        }
    }

    pub fn source_builder(&self) -> AudioItemBuilder {
        Self::builder(Time::ZERO, self.source_duration)
            .id(Uuid::nil())
            .source_duration(self.source_duration)
            .repeat_strategy(RepeatStrategy::Empty)
            .speed_method(AudioSpeedMethod::Naive)
            .source(self.source.clone())
            .track_id(self.track_id)
            .file(self.file.clone())
    }

    pub fn transition_gain(&self, position: Time) -> f32 {
        let transitions = [
            self.transitions.intro.as_ref().map(|transition| {
                (
                    TransitionSide::Intro,
                    transition.duration,
                    transition.interpolation,
                )
            }),
            self.transitions.outro.as_ref().map(|transition| {
                (
                    TransitionSide::Outro,
                    transition.duration,
                    transition.interpolation,
                )
            }),
        ]
        .into_iter()
        .flatten();
        shrimply_math_media::audio_transition_gain(self.start, self.end, transitions, position)
    }
}

pub struct AudioItemBuilder {
    item: AudioItem,
}

impl AudioItemBuilder {
    pub fn id(mut self, id: Uuid) -> Self {
        self.item.id = id;
        self
    }

    pub fn time_offset(mut self, time_offset: Time) -> Self {
        self.item.time_offset = time_offset;
        self
    }

    pub fn source_duration(mut self, source_duration: Time) -> Self {
        self.item.source_duration = source_duration;
        self
    }

    pub fn playback_speed(mut self, playback_speed: Fraction) -> Self {
        self.item.playback_speed = playback_speed;
        self
    }

    pub fn repeat_strategy(mut self, repeat_strategy: RepeatStrategy) -> Self {
        self.item.repeat_strategy = repeat_strategy;
        self
    }

    pub fn speed_method(mut self, speed_method: AudioSpeedMethod) -> Self {
        self.item.speed_method = speed_method;
        self
    }

    pub fn gain(mut self, gain: GainModifier) -> Self {
        self.item.gain = Box::new(gain);
        self
    }

    pub fn group_id(mut self, group_id: Option<u64>) -> Self {
        self.item.group_id = group_id;
        self
    }

    pub fn source(mut self, source: AudioSource) -> Self {
        self.item.source = source;
        self
    }

    pub fn track_id(mut self, track_id: u32) -> Self {
        self.item.track_id = track_id;
        self
    }

    pub fn modifiers(mut self, modifiers: Vec<AudioModifier>) -> Self {
        self.item.modifiers = modifiers;
        self
    }

    pub fn file(mut self, file: impl Into<Asset>) -> Self {
        self.item.file = file.into();
        self
    }

    pub fn build(self) -> AudioItem {
        self.item
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VisualTransitions {
    pub intro: Option<VisualTransition>,
    pub outro: Option<VisualTransition>,
    #[serde(default)]
    pub to_next: Option<VisualClipTransition>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AudioTransitions {
    pub intro: Option<AudioTransition>,
    pub outro: Option<AudioTransition>,
    #[serde(default)]
    pub to_next: Option<Box<AudioClipTransition>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct VisualClipTransition {
    pub target_item_id: Uuid,
    pub duration: Time,
    pub kind: VisualClipTransitionKind,
    #[serde(default = "default_visual_clip_transition_interpolation")]
    pub interpolation: Interpolation,
    #[serde(default)]
    pub direction_degrees: f32,
    #[serde(default = "default_visual_clip_transition_softness")]
    pub softness: f32,
    #[serde(default = "default_transition_iris_center")]
    pub center: glam::Vec2,
    #[serde(default = "default_true")]
    pub iris_from_inside: bool,
    #[serde(default = "default_true")]
    pub clockwise: bool,
    #[serde(default)]
    pub fade_color: Color<u8>,
    #[serde(default = "default_visual_clip_transition_dissolve_grain_size")]
    pub dissolve_grain_size: u32,
    #[serde(default)]
    pub zoom_start_scale: f32,
    #[serde(default)]
    pub fade_opacity: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioClipTransition {
    pub target_item_id: Uuid,
    pub duration: Time,
    pub curve: AudioClipTransitionCurve,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualClipTransitionKind {
    #[default]
    CrossFade,
    #[serde(alias = "fade_through_white")]
    FadeThroughColor,
    Wipe,
    Morph,
    Iris,
    ClockWipe,
    Dissolve,
    Slide,
    Push,
    Zoom,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VisualTransition {
    pub duration: Time,
    pub kind: VisualTransitionKind,
    pub interpolation: Interpolation,
    pub slide_rotation_degrees: f32,
    pub slide_distance: f32,
    pub write_ordering: WriteOrdering,
    #[serde(default)]
    pub drawing_stroke_overlap: f32,
    #[serde(default)]
    pub drawing_stroke_length_weight: f32,
    #[serde(default)]
    pub drawing_fill_mode: DrawingFillMode,
    #[serde(default)]
    pub morph_unit: MorphUnit,
    #[serde(default = "default_transition_effect_amount")]
    pub effect_amount: f32,
    #[serde(default = "default_transition_effect_detail")]
    pub effect_detail: f32,
    #[serde(default)]
    pub effect_angle_degrees: f32,
    #[serde(default)]
    pub effect_softness: f32,
    #[serde(default = "default_transition_iris_center")]
    pub iris_center: glam::Vec2,
    #[serde(default = "default_true")]
    pub effect_fade: bool,
    #[serde(default)]
    pub effect_evolve_seed: bool,
    #[serde(default = "default_transition_seed_frequency")]
    pub effect_seed_frequency: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioTransition {
    pub duration: Time,
    pub kind: AudioTransitionKind,
    pub interpolation: Interpolation,
}

#[derive(
    Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VisualTransitionKind {
    #[default]
    Fade,
    Slide,
    SlideFade,
    Morph,
    Write,
    Drawing,
    Create,
    FacetAssembly,
    Coalesce,
    ContourCurrent,
    SoftRefraction,
    MorphologicalResolve,
    LivingFill,
    Diffusion,
    ReverseDiffusion,
    Wipe,
    Iris,
    ClockWipe,
    Zoom,
    Spin,
    Blur,
    Pixelate,
    Dissolve,
    TriangularFold,
    Origami,
    StreakWipe,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTransitionKind {
    #[default]
    Fade,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteOrdering {
    #[default]
    Sequential,
    Simultaneous,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrawingFillMode {
    #[default]
    FadeTogether,
    FadeSequentially,
    Direct,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MorphUnit {
    #[default]
    Letter,
    Word,
}

impl VisualTransition {
    pub fn new(side: TransitionSide, duration: Time, canvas_size: CanvasSize) -> Self {
        Self {
            duration,
            kind: VisualTransitionKind::Fade,
            interpolation: visual_transition_default_interpolation(
                side,
                VisualTransitionKind::Fade,
            ),
            slide_rotation_degrees: 90.0,
            slide_distance: canvas_size.height.max(1) as f32,
            write_ordering: WriteOrdering::Sequential,
            drawing_stroke_overlap: 0.0,
            drawing_stroke_length_weight: 0.0,
            drawing_fill_mode: DrawingFillMode::FadeTogether,
            morph_unit: MorphUnit::Letter,
            effect_amount: default_transition_effect_amount(),
            effect_detail: default_transition_effect_detail(),
            effect_angle_degrees: 0.0,
            effect_softness: 0.0,
            iris_center: default_transition_iris_center(),
            effect_fade: true,
            effect_evolve_seed: false,
            effect_seed_frequency: default_transition_seed_frequency(),
        }
    }

    pub fn set_kind(&mut self, side: TransitionSide, kind: VisualTransitionKind) {
        self.kind = kind;
        self.interpolation = visual_transition_default_interpolation(side, kind);
        (
            self.effect_amount,
            self.effect_detail,
            self.effect_angle_degrees,
            self.effect_softness,
        ) = visual_transition_effect_defaults(kind);
        self.effect_fade = true;
        self.effect_evolve_seed = false;
        self.effect_seed_frequency = default_transition_seed_frequency();
    }
}

impl VisualClipTransition {
    pub fn new(target_item_id: Uuid, duration: Time) -> Self {
        Self {
            target_item_id,
            duration,
            kind: VisualClipTransitionKind::CrossFade,
            interpolation: visual_clip_transition_default_interpolation(
                VisualClipTransitionKind::CrossFade,
            ),
            direction_degrees: 0.0,
            softness: default_visual_clip_transition_softness(),
            center: default_transition_iris_center(),
            iris_from_inside: true,
            clockwise: true,
            fade_color: Color::<u8>::WHITE,
            dissolve_grain_size: default_visual_clip_transition_dissolve_grain_size(),
            zoom_start_scale: 0.0,
            fade_opacity: false,
        }
    }

    pub fn set_kind(&mut self, kind: VisualClipTransitionKind) {
        self.kind = kind;
        self.interpolation = visual_clip_transition_default_interpolation(kind);
        self.direction_degrees = if kind == VisualClipTransitionKind::ClockWipe {
            -90.0
        } else {
            0.0
        };
        self.softness = if kind == VisualClipTransitionKind::ClockWipe {
            0.02
        } else {
            default_visual_clip_transition_softness()
        };
        self.center = default_transition_iris_center();
        self.iris_from_inside = true;
        self.clockwise = true;
        self.fade_color = Color::<u8>::WHITE;
        self.dissolve_grain_size = default_visual_clip_transition_dissolve_grain_size();
        self.zoom_start_scale = 0.0;
        self.fade_opacity = kind == VisualClipTransitionKind::Zoom;
    }
}

pub fn visual_clip_transition_default_interpolation(
    kind: VisualClipTransitionKind,
) -> Interpolation {
    match kind {
        VisualClipTransitionKind::Morph => Interpolation::ManimSmooth,
        VisualClipTransitionKind::Iris
        | VisualClipTransitionKind::Slide
        | VisualClipTransitionKind::Push
        | VisualClipTransitionKind::Zoom => Interpolation::SineInOut,
        VisualClipTransitionKind::CrossFade
        | VisualClipTransitionKind::FadeThroughColor
        | VisualClipTransitionKind::Wipe
        | VisualClipTransitionKind::ClockWipe
        | VisualClipTransitionKind::Dissolve => Interpolation::Linear,
    }
}

fn default_visual_clip_transition_interpolation() -> Interpolation {
    Interpolation::Linear
}

fn default_visual_clip_transition_softness() -> f32 {
    0.05
}

fn default_visual_clip_transition_dissolve_grain_size() -> u32 {
    4
}

fn default_transition_effect_amount() -> f32 {
    1.0
}

fn default_transition_effect_detail() -> f32 {
    1.0
}

fn default_transition_iris_center() -> glam::Vec2 {
    glam::Vec2::splat(0.5)
}

fn default_transition_seed_frequency() -> u32 {
    12
}

pub fn visual_transition_effect_defaults(kind: VisualTransitionKind) -> (f32, f32, f32, f32) {
    match kind {
        VisualTransitionKind::Coalesce => (1.0, 3.0, 0.0, 0.0),
        VisualTransitionKind::ContourCurrent => (1.0, 0.24, 0.0, 0.0),
        VisualTransitionKind::SoftRefraction => (1.0, 1.0, 0.0, 0.0),
        VisualTransitionKind::MorphologicalResolve => (1.0, 0.5, 0.0, 0.0),
        VisualTransitionKind::LivingFill => (0.16, 0.5, 12.0, 0.0),
        VisualTransitionKind::Diffusion | VisualTransitionKind::ReverseDiffusion => {
            (1.0, 1.0, 0.0, 0.0)
        }
        VisualTransitionKind::Wipe => (1.0, 0.05, 0.0, 0.0),
        VisualTransitionKind::Iris => (1.0, 0.05, 0.0, 0.0),
        VisualTransitionKind::ClockWipe => (1.0, 0.02, -90.0, 0.0),
        VisualTransitionKind::Zoom => (0.0, 1.0, 0.0, 0.0),
        VisualTransitionKind::Spin => (0.0, 1.0, 360.0, 0.0),
        VisualTransitionKind::Blur => (24.0, 1.0, 0.0, 0.0),
        VisualTransitionKind::Pixelate => (48.0, 1.0, 0.0, 0.0),
        VisualTransitionKind::Dissolve => (1.0, 4.0, 0.0, 0.0),
        VisualTransitionKind::TriangularFold => (0.85, 144.0, 25.0, 0.0),
        VisualTransitionKind::Origami => (0.95, 4.0, 25.0, 0.0),
        VisualTransitionKind::StreakWipe => (0.65, 12.0, 0.0, 2.0),
        _ => (1.0, 1.0, 0.0, 0.0),
    }
}

impl AudioTransition {
    pub fn new(_side: TransitionSide, duration: Time) -> Self {
        Self {
            duration,
            kind: AudioTransitionKind::Fade,
            interpolation: Interpolation::Linear,
        }
    }
}

pub fn visual_transition_default_interpolation(
    side: TransitionSide,
    kind: VisualTransitionKind,
) -> Interpolation {
    match kind {
        VisualTransitionKind::Fade
        | VisualTransitionKind::Write
        | VisualTransitionKind::Drawing => Interpolation::Linear,
        VisualTransitionKind::Morph => Interpolation::ManimSmooth,
        VisualTransitionKind::Create => Interpolation::ManimSmooth,
        VisualTransitionKind::FacetAssembly => Interpolation::CubicOut,
        VisualTransitionKind::Coalesce => Interpolation::SineInOut,
        VisualTransitionKind::ContourCurrent => Interpolation::SineInOut,
        VisualTransitionKind::SoftRefraction => Interpolation::SineInOut,
        VisualTransitionKind::MorphologicalResolve => Interpolation::CubicOut,
        VisualTransitionKind::LivingFill => Interpolation::SineInOut,
        VisualTransitionKind::Blur
        | VisualTransitionKind::Pixelate
        | VisualTransitionKind::TriangularFold
        | VisualTransitionKind::Origami => Interpolation::SineInOut,
        VisualTransitionKind::Dissolve | VisualTransitionKind::ClockWipe => Interpolation::Linear,
        VisualTransitionKind::Diffusion | VisualTransitionKind::ReverseDiffusion => {
            Interpolation::SineInOut
        }
        _ => match side {
            TransitionSide::Intro => Interpolation::ExpoOut,
            TransitionSide::Outro => Interpolation::ExpoIn,
        },
    }
}

pub type Transform = Transform2D<TimelineValue<glam::Vec2>, TimelineValue<f32>>;

impl VideoItem {
    pub fn source_visual_kind(&self) -> shrimply_visual_modifiers::VisualKind {
        match self.content {
            VideoItemContent::Svg
            | VideoItemContent::Text(_)
            | VideoItemContent::Shape(_)
            | VideoItemContent::Paint(_) => shrimply_visual_modifiers::VisualKind::Vector,
            VideoItemContent::Manim(_) => shrimply_visual_modifiers::VisualKind::Manim,
            VideoItemContent::Background(_) => shrimply_visual_modifiers::VisualKind::Background,
            VideoItemContent::Media
            | VideoItemContent::Image
            | VideoItemContent::Gif
            | VideoItemContent::Pdf(_)
            | VideoItemContent::Blender(_)
            | VideoItemContent::LayeredImage(_)
            | VideoItemContent::FoldedSequence(_) => shrimply_visual_modifiers::VisualKind::Raster,
            VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_) => {
                shrimply_visual_modifiers::VisualKind::Scene3d
            }
        }
    }

    pub fn modifier_output_kind(&self) -> Result<shrimply_visual_modifiers::VisualKind, String> {
        self.modifier_output_state().map(|state| state.kind)
    }

    pub fn modifier_output_state(
        &self,
    ) -> Result<shrimply_visual_modifiers::ModifierState, String> {
        self.modifier_output_state_for(&self.modifiers)
    }

    pub fn modifier_output_state_for(
        &self,
        modifiers: &[VisualModifier],
    ) -> Result<shrimply_visual_modifiers::ModifierState, String> {
        let source = match self.content {
            VideoItemContent::Image => shrimply_visual_modifiers::ModifierSource::Image,
            VideoItemContent::Paint(_) => shrimply_visual_modifiers::ModifierSource::Paint,
            VideoItemContent::Text(_) => shrimply_visual_modifiers::ModifierSource::Text,
            VideoItemContent::Obj(_) => shrimply_visual_modifiers::ModifierSource::Obj,
            _ => shrimply_visual_modifiers::ModifierSource::Other,
        };
        shrimply_visual_modifiers::chain_output(
            shrimply_visual_modifiers::ModifierState {
                source,
                kind: self.source_visual_kind(),
                pristine: true,
            },
            modifiers
                .iter()
                .map(|modifier| (modifier.enabled, &modifier.effect)),
        )
    }

    pub fn rendered_canvas_size(&self, native: CanvasSize) -> CanvasSize {
        match self
            .modifier_output_kind()
            .unwrap_or_else(|_| self.source_visual_kind())
        {
            shrimply_visual_modifiers::VisualKind::Vector
            | shrimply_visual_modifiers::VisualKind::Manim
            | shrimply_visual_modifiers::VisualKind::Background
            | shrimply_visual_modifiers::VisualKind::Scene3d => {
                self.render_canvas_size.unwrap_or(native)
            }
            shrimply_visual_modifiers::VisualKind::Raster => native,
        }
    }

    pub fn natural_transform(&self, canvas_size: CanvasSize) -> Transform {
        if matches!(self.content, VideoItemContent::Background(_)) {
            return Transform::fill(canvas_size);
        }
        if matches!(self.content, VideoItemContent::FoldedSequence(_)) {
            return Transform::fill(canvas_size);
        }
        if !self.is_video_media() {
            return Transform::natural_size(canvas_size, self.source_width, self.source_height);
        }
        let canvas = Vec2::new(
            canvas_size.width.max(1) as f32,
            canvas_size.height.max(1) as f32,
        );
        video_item_media_size(self, canvas)
            .map(|media| {
                let size = media_size_components(media);
                Transform::natural_size(canvas_size, size.x, size.y)
            })
            .unwrap_or_else(|| Transform::fill(canvas_size))
    }
}

fn default_true() -> bool {
    true
}

fn default_project_name() -> String {
    "Untitled Project".to_string()
}

fn current_project_format_version() -> u32 {
    PROJECT_FORMAT_VERSION
}

fn default_project_fps() -> Fraction {
    DEFAULT_PROJECT_FPS
}

fn default_canvas_size() -> CanvasSize {
    DEFAULT_CANVAS_SIZE
}

fn ensure_unique_id(id: &mut Uuid, seen: &mut HashSet<Uuid>) {
    if id.is_nil() || !seen.insert(*id) {
        loop {
            *id = Uuid::new_v4();
            if seen.insert(*id) {
                break;
            }
        }
    }
}

fn ensure_video_property_ids(item: &mut VideoItem, seen: &mut HashSet<Uuid>) {
    ensure_transform_ids(&mut item.transform, seen);
    ensure_timeline_value_ids(&mut item.compositing.opacity, seen);
    ensure_timeline_value_ids(&mut item.compositing.blend_mode, seen);
    if let Some(mask) = &mut item.compositing.alpha_mask {
        ensure_alpha_mask_ids(mask, seen);
    }
    ensure_timeline_value_ids(&mut item.visibility, seen);
    ensure_timeline_value_ids(&mut item.sample_method, seen);
    for modifier in &mut item.modifiers {
        ensure_unique_id(&mut modifier.id, seen);
        modifier.effect.ensure_ids(seen);
        if let Some(mask) = &mut modifier.alpha_mask {
            ensure_alpha_mask_ids(mask, seen);
        }
    }
    if let Some(transform) = &mut item.default_transform {
        ensure_transform_ids(transform, seen);
    }
    match &mut item.content {
        VideoItemContent::LayeredImage(image) => {
            for layer in &mut image.layers {
                ensure_unique_id(&mut layer.id, seen);
                if let Some(visibility) = &mut layer.visibility {
                    ensure_timeline_value_ids(visibility, seen);
                }
            }
        }
        VideoItemContent::Text(text) => {
            ensure_timeline_value_ids(&mut text.font_style, seen);
            ensure_timeline_value_ids(&mut text.h_align, seen);
            ensure_timeline_value_ids(&mut text.v_align, seen);
            ensure_timeline_value_ids(&mut text.direction, seen);
            for value in [
                &mut text.font_size,
                &mut text.font_weight,
                &mut text.tracking,
                &mut text.line_height,
                &mut text.background_roundness,
                &mut text.outline_width,
                &mut text.shadow_distance,
                &mut text.shadow_direction_degrees,
                &mut text.shadow_width,
                &mut text.shadow_blur,
            ] {
                ensure_timeline_value_ids(value, seen);
            }
            ensure_timeline_value_ids(&mut text.text, seen);
            ensure_timeline_value_ids(&mut text.background_padding, seen);
            for value in [
                &mut text.color,
                &mut text.background_color,
                &mut text.outline_color,
                &mut text.shadow_color,
            ] {
                ensure_timeline_value_ids(value, seen);
            }
        }
        VideoItemContent::Shape(shape) => {
            ensure_timeline_value_ids(&mut shape.shape, seen);
            ensure_timeline_value_ids(&mut shape.rounding_strategy, seen);
            ensure_timeline_value_ids(&mut shape.size, seen);
            ensure_timeline_value_ids(&mut shape.star_points, seen);
            for value in [
                &mut shape.star_inner_radius_percent,
                &mut shape.arrow_shaft_width_percent,
                &mut shape.arrow_head_length_percent,
                &mut shape.cross_arm_thickness_percent,
                &mut shape.ellipse_inner_radius_percent,
                &mut shape.ellipse_completion_degrees,
                &mut shape.outline_width,
                &mut shape.corner_radius,
                &mut shape.shadow_distance,
                &mut shape.shadow_direction_degrees,
                &mut shape.shadow_width,
                &mut shape.shadow_blur,
            ] {
                ensure_timeline_value_ids(value, seen);
            }
            for value in [
                &mut shape.fill,
                &mut shape.outline_color,
                &mut shape.shadow_color,
            ] {
                ensure_timeline_value_ids(value, seen);
            }
        }
        VideoItemContent::Paint(paint) => {
            ensure_timeline_value_ids(&mut paint.drawing, seen);
            match &mut paint.drawing.base {
                TimelineBase::Const(drawing) => paint::ensure_drawing_ids(drawing, seen),
                TimelineBase::Keyframes(keyframes) => {
                    for keyframe in keyframes {
                        paint::ensure_drawing_ids(&mut keyframe.value, seen);
                    }
                }
            }
            for entry in &mut paint.palette {
                ensure_timeline_value_ids(&mut entry.color, seen);
                if let Some(texture) = &mut entry.texture {
                    ensure_timeline_value_ids(&mut texture.repeat_scale, seen);
                    ensure_timeline_value_ids(&mut texture.rotation_degrees, seen);
                }
            }
            for value in [
                &mut paint.stroke.width,
                &mut paint.stroke.thinning,
                &mut paint.stroke.smoothing,
                &mut paint.stroke.streamline,
                &mut paint.stroke.simplification_tolerance,
                &mut paint.stroke.maximum_subdivision_spacing,
                &mut paint.stroke.start.taper_distance,
                &mut paint.stroke.end.taper_distance,
            ] {
                ensure_timeline_value_ids(value, seen);
            }
            ensure_timeline_value_ids(&mut paint.stroke.start.cap, seen);
            ensure_timeline_value_ids(&mut paint.stroke.start.taper, seen);
            ensure_timeline_value_ids(&mut paint.stroke.end.cap, seen);
            ensure_timeline_value_ids(&mut paint.stroke.end.taper, seen);
            ensure_timeline_value_ids(&mut paint.fill.closure_tolerance, seen);
            ensure_transform_ids(&mut paint.stroke_transform, seen);
        }
        VideoItemContent::Obj(scene) => ensure_obj_scene_ids(scene, seen),
        VideoItemContent::Gaussian(scene) => ensure_gaussian_scene_ids(scene, seen),
        VideoItemContent::Media
        | VideoItemContent::Image
        | VideoItemContent::Gif
        | VideoItemContent::Svg
        | VideoItemContent::Pdf(_)
        | VideoItemContent::Manim(_)
        | VideoItemContent::Blender(_)
        | VideoItemContent::FoldedSequence(_) => {}
        VideoItemContent::Background(background) => background.generator.ensure_ids(seen),
    }
}

pub fn ensure_alpha_mask_ids(mask: &mut VisualAlphaMask, seen: &mut HashSet<Uuid>) {
    ensure_timeline_value_ids(&mut mask.center, seen);
    ensure_timeline_value_ids(&mut mask.size, seen);
    ensure_timeline_value_ids(&mut mask.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut mask.feather, seen);
    ensure_timeline_value_ids(&mut mask.rounding, seen);
}

fn ensure_audio_property_ids(item: &mut AudioItem, seen: &mut HashSet<Uuid>) {
    item.gain.ensure_ids(seen);
    if let AudioSource::Generator(generator) = &mut item.source {
        generator.ensure_ids(seen);
    }
    for modifier in &mut item.modifiers {
        ensure_unique_id(&mut modifier.id, seen);
        modifier.effect.ensure_ids(seen);
    }
}

fn ensure_transform_ids(transform: &mut Transform, seen: &mut HashSet<Uuid>) {
    ensure_unique_id(&mut transform.id, seen);
    ensure_timeline_value_ids(&mut transform.position, seen);
    ensure_timeline_value_ids(&mut transform.anchor, seen);
    ensure_timeline_value_ids(&mut transform.scale, seen);
    ensure_timeline_value_ids(&mut transform.shear, seen);
    ensure_timeline_value_ids(&mut transform.rotation_degrees, seen);
}

fn ensure_animated_vec3_ids(value: &mut AnimatedVec3, seen: &mut HashSet<Uuid>) {
    ensure_timeline_value_ids(value, seen);
}

fn ensure_obj_scene_ids(scene: &mut ObjScene, seen: &mut HashSet<Uuid>) {
    ensure_animated_vec3_ids(&mut scene.model.position, seen);
    ensure_animated_vec3_ids(&mut scene.model.anchor, seen);
    ensure_animated_vec3_ids(&mut scene.model.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.model.rotation_order, seen);
    ensure_animated_vec3_ids(&mut scene.model.scale, seen);
    ensure_animated_vec3_ids(&mut scene.camera.position, seen);
    ensure_animated_vec3_ids(&mut scene.camera.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.camera.vertical_fov_degrees, seen);
    ensure_timeline_value_ids(&mut scene.camera.orthographic_height, seen);
    ensure_timeline_value_ids(&mut scene.camera.focus_distance, seen);
    ensure_timeline_value_ids(&mut scene.camera.background_distance, seen);
    ensure_timeline_value_ids(&mut scene.camera.background_intensity, seen);
    ensure_timeline_value_ids(&mut scene.camera.f_stop, seen);
    ensure_timeline_value_ids(&mut scene.camera.exposure_ev, seen);
    ensure_timeline_value_ids(&mut scene.material.base_color, seen);
    ensure_timeline_value_ids(&mut scene.material.metallic, seen);
    ensure_timeline_value_ids(&mut scene.material.roughness, seen);
    ensure_timeline_value_ids(&mut scene.material.subsurface, seen);
    ensure_timeline_value_ids(&mut scene.material.clearcoat, seen);
    ensure_timeline_value_ids(&mut scene.material.sheen, seen);
    ensure_timeline_value_ids(&mut scene.material.transmission, seen);
    ensure_timeline_value_ids(&mut scene.material.ior, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.bands, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.color_levels, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.kuwahara_radius, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.kuwahara_strength, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_color, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_strength, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_darkest_tone, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_dot_size, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_dot_density, seen);
    ensure_timeline_value_ids(
        &mut scene.material.toon.shadow_dot_distribution_randomness,
        seen,
    );
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_dot_size_randomness, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_line_direction_degrees, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_line_width, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_line_density, seen);
    ensure_timeline_value_ids(
        &mut scene.material.toon.shadow_line_distribution_randomness,
        seen,
    );
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_line_width_randomness, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.shadow_pattern_softness, seen);
    ensure_timeline_value_ids(
        &mut scene.material.toon.shadow_crosshatch_angle_degrees,
        seen,
    );
    ensure_timeline_value_ids(
        &mut scene.material.toon.shadow_crosshatch_max_directions,
        seen,
    );
    ensure_timeline_value_ids(&mut scene.material.toon.rim_color, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.rim_strength, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.rim_power, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.specular_size, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.specular_strength, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.color, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.width, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.opacity, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.depth_threshold, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.normal_angle_degrees, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.dog_inner_radius, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.dog_radius_ratio, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.dog_threshold, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.dog_sharpness, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.offset_variation, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.width_variation, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.offset_frequency, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.width_frequency, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.aggressiveness, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.noise_seed, seen);
    ensure_timeline_value_ids(&mut scene.material.toon.outline.noise_evolution, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.enabled, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.intensity, seen);
    ensure_animated_vec3_ids(&mut scene.shadow_receiver.position, seen);
    ensure_animated_vec3_ids(&mut scene.shadow_receiver.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.opacity, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.shadow_strength, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.reflection, seen);
    ensure_timeline_value_ids(&mut scene.shadow_receiver.roughness, seen);
    ensure_timeline_value_ids(&mut scene.environment.solid_color, seen);
    ensure_animated_vec3_ids(&mut scene.environment.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.environment.intensity, seen);
}

fn ensure_gaussian_scene_ids(
    scene: &mut shrimply_3dgs_core::GaussianScene,
    seen: &mut HashSet<Uuid>,
) {
    ensure_animated_vec3_ids(&mut scene.model.position, seen);
    ensure_animated_vec3_ids(&mut scene.model.anchor, seen);
    ensure_animated_vec3_ids(&mut scene.model.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.model.rotation_order, seen);
    ensure_animated_vec3_ids(&mut scene.model.scale, seen);
    ensure_animated_vec3_ids(&mut scene.camera.position, seen);
    ensure_animated_vec3_ids(&mut scene.camera.rotation_degrees, seen);
    ensure_timeline_value_ids(&mut scene.camera.vertical_fov_degrees, seen);
    ensure_timeline_value_ids(&mut scene.camera.orthographic_height, seen);
    ensure_timeline_value_ids(&mut scene.camera.focus_distance, seen);
    ensure_timeline_value_ids(&mut scene.camera.f_stop, seen);
    ensure_timeline_value_ids(&mut scene.camera.exposure_ev, seen);
}

fn ensure_timeline_value_ids<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    seen: &mut HashSet<Uuid>,
) {
    ensure_unique_id(&mut value.id, seen);
    if let Some(expression) = &mut value.expression {
        ensure_unique_id(&mut expression.id, seen);
    }
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return;
    };
    for keyframe in keyframes {
        ensure_unique_id(keyframe.id_mut(), seen);
    }
}

#[derive(Clone, Copy)]
enum MediaTransformNormalization {
    CanvasAnchor,
    LegacyCanvasTransform,
}

fn media_transform_normalization(
    transform: &Transform,
    canvas: Vec2,
    media: Vec2,
) -> Option<MediaTransformNormalization> {
    if !vec2_nearly_eq(transform.anchor.fallback(), canvas * 0.5) {
        return None;
    }

    let scale = transform.scale.fallback();
    if vec2_nearly_eq(scale, media / canvas) {
        return Some(MediaTransformNormalization::LegacyCanvasTransform);
    }
    if vec2_nearly_eq(scale, Vec2::ONE) {
        return Some(MediaTransformNormalization::CanvasAnchor);
    }
    None
}

fn video_item_media_size(item: &VideoItem, canvas: Vec2) -> Option<Vec2> {
    if item.source_width > 0 && item.source_height > 0 {
        return Some(Vec2::new(
            item.source_width as f32,
            item.source_height as f32,
        ));
    }

    item.default_transform
        .as_ref()
        .and_then(|transform| infer_legacy_media_size(transform, canvas))
        .or_else(|| infer_legacy_media_size(&item.transform, canvas))
}

fn infer_legacy_media_size(transform: &Transform, canvas: Vec2) -> Option<Vec2> {
    if !vec2_nearly_eq(transform.anchor.fallback(), canvas * 0.5) {
        return None;
    }
    let scale = transform.scale.fallback();
    if scale.x <= 0.0 || scale.y <= 0.0 {
        return None;
    }
    let media = canvas * scale;
    (media.x.is_finite() && media.y.is_finite()).then_some(media.max(Vec2::ONE))
}

fn media_size_components(media: Vec2) -> UVec2 {
    UVec2::new(media_size_component(media.x), media_size_component(media.y))
}

fn media_size_component(value: f32) -> u32 {
    value.round().clamp(1.0, u32::MAX as f32) as u32
}

fn normalize_transform_to_media(
    transform: &mut Transform,
    mode: MediaTransformNormalization,
    canvas: Vec2,
    media: Vec2,
) {
    scale_timeline_vector(&mut transform.anchor, media / canvas);
    if matches!(mode, MediaTransformNormalization::LegacyCanvasTransform) {
        scale_timeline_vector(&mut transform.scale, canvas / media);
    }
}

fn scale_timeline_vector(value: &mut TimelineValue<glam::Vec2>, factor: Vec2) {
    match &mut value.base {
        TimelineBase::Const(value) => *value *= factor,
        TimelineBase::Keyframes(keyframes) => {
            for keyframe in keyframes {
                keyframe.value *= factor;
            }
        }
    }
}

fn vec2_nearly_eq(left: Vec2, right: Vec2) -> bool {
    (left - right).abs().cmple(Vec2::splat(0.01)).all()
}
