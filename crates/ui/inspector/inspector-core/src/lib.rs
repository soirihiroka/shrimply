pub mod alpha_mask;
mod audio_cache;
pub mod audio_generator;
mod audio_modifiers;
pub mod background;
pub mod benchmarking;
pub mod camera_source;
pub mod caption;
pub mod document;
pub mod file_selection;
pub mod font_cache;
pub mod font_selector;
pub mod gaussian_3d;
pub mod generated;
pub mod info;
pub mod item;
pub mod keyframe_graph;
pub mod keyframe_model;
mod layered;
pub mod list;
#[path = "video/manim_parameters.rs"]
pub mod manim_parameters;
mod model;
mod model_catalog;
pub mod paint;
pub mod project;
mod refresh;
pub mod rhai_editor;
pub mod scene_3d;
mod section;
pub mod selector;
mod target;
mod timeline;
pub mod timeline_color;
pub mod timeline_text;
pub mod timeline_value;
pub mod track;
pub mod transform;
pub mod transition;
pub mod tts;
mod value;
pub mod video;
pub mod visual_modifiers;

pub use alpha_mask::AlphaMaskPresentation;
pub use audio_cache::{
    AudioCachePresentation, AudioCachePreset, CacheStatusPoll, CacheStatusTracker,
    audio_cache_control, audio_cache_presentation, audio_cache_preset, audio_cache_status,
};
pub use audio_modifiers::{
    AudioModifierControl, AudioModifierOption, AudioModifierScalarPresentation,
    audio_modifier_catalog, audio_modifier_controls, cached_voice_change_models,
    default_audio_modifier_effect, same_audio_modifier_effect, voice_change_model_catalog,
    voice_change_models,
};
pub use camera_source::CameraSourcePresentation;
pub use document::BasicInspectorAction;
pub use info::InspectorMedia;
pub use keyframe_graph::{GraphPoint, GraphSegment, InspectorGraphKind, ScalarGraph};
pub use layered::LayeredState;
pub use model::{
    AudioCacheStatus, AudioModifierChoice, AudioModifierKeyframeMove, CacheControlPresentation,
    CacheStatus, CameraAnalysis, INSPECTOR_MIN_WIDTH, InspectorAnalysisBackend,
    InspectorCapabilities, InspectorCommit, InspectorController, InspectorDetail,
    InspectorExpressionOutput, InspectorRuntime, InspectorSnapshot, TimelineModeChange,
    TransparentFillAnalysis, VisualCacheBake, VisualCacheStatus, cache_control_presentation,
};
pub use project::ProjectPresentation;
pub use section::{
    AnalysisControlPresentation, AnalysisTooltip, ControlKind, ControlRowRole, InspectorControl,
    InspectorControlAction, InspectorSection, KeyframeCommits, NumberConstraint, NumberMapping,
    NumberSpec, ScalarStorage, TextKeyframeCommits,
};
pub use target::InspectorTarget;
pub use track::TrackPresentation;
pub use transition::{TransitionPresentation, TransitionType};
pub use video::{
    VideoCard, VideoCardScope, VideoPresentation, VideoReset, VideoStreamPresentation,
};
pub use visual_modifiers::{
    OpacityModifierPresentation, TransformModifierPresentation,
    VisualModifierAlphaMaskPresentation, VisualModifierBodyPresentation, VisualModifierChoice,
    VisualModifierPresentation, default_visual_modifier_effect, sam2_analysis_control,
    visual_cache_status, visual_modifier_catalog, visual_modifier_presentations,
};

pub mod voice_models;

mod trust;
