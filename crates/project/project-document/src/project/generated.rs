use hashbrown::HashMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    Asset, CanvasSize, Color, RepeatStrategy, ResolvedTransform, SequenceReference, Time,
    Transform, VerticalAlign, VisualCompositing, VisualItem, default_playback_speed,
};
use shrimply_3dgs_core::GaussianScene;
use shrimply_paint_model::PaintItem;
use shrimply_property_model::timeline_value::*;
pub use shrimply_property_model::{
    DEFAULT_TEXT_FONT_FAMILY, FontFamily, FontVariation, TextDirection, TextFontStyle,
    TextHorizontalAlign,
};
use shrimply_scene_3d::ObjScene;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum VisualSource {
    #[default]
    Media,
    Image,
    Gif,
    Svg,
    Pdf(Box<PdfItem>),
    Manim(Box<ManimItem>),
    Blender(Box<BlenderItem>),
    #[serde(alias = "psd")]
    LayeredImage(Box<LayeredImageItem>),
    Text(Box<TextItem>),
    Shape(Box<ShapeItem>),
    Paint(Box<PaintItem>),
    Background(Box<shrimply_background::Background>),
    Obj(Box<ObjScene>),
    Gaussian(Box<GaussianScene>),
    FoldedSequence(SequenceReference),
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PdfItem {
    #[serde(default)]
    pub page: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ManimItem {
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub parameters: HashMap<String, ManimParameterValue>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlenderItem {
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub view_layer: String,
    #[serde(default)]
    pub camera: String,
    #[serde(default)]
    pub render_method: BlenderRenderMethod,
    #[serde(default = "default_blender_preview_render_method")]
    pub preview_render_method: BlenderRenderMethod,
    #[serde(default)]
    pub preview_downsample: BlenderPreviewDownsample,
}

impl Default for BlenderItem {
    fn default() -> Self {
        Self {
            scene: String::new(),
            view_layer: String::new(),
            camera: String::new(),
            render_method: BlenderRenderMethod::SceneRenderer,
            preview_render_method: BlenderRenderMethod::MaterialPreview,
            preview_downsample: BlenderPreviewDownsample::X2,
        }
    }
}

const fn default_blender_preview_render_method() -> BlenderRenderMethod {
    BlenderRenderMethod::MaterialPreview
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlenderRenderMethod {
    #[serde(alias = "fast_preview", alias = "workbench")]
    Solid,
    MaterialPreview,
    #[default]
    SceneRenderer,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlenderPreviewDownsample {
    Full,
    #[default]
    X2,
    X4,
    X8,
    X16,
    X32,
}

impl BlenderPreviewDownsample {
    pub const fn factor(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::X2 => 2,
            Self::X4 => 4,
            Self::X8 => 8,
            Self::X16 => 16,
            Self::X32 => 32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ManimParameterValue {
    Integer(i64),
    Float(f64),
    Fraction { numerator: i64, denominator: i64 },
    Color(Color<u8>),
    Option(String),
    Boolean(bool),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ManimParameter {
    pub key: String,
    pub label: String,
    pub default: ManimParameterValue,
    pub value: ManimParameterValue,
    pub control: ManimParameterControl,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ManimParameterControl {
    AntiAliasing,
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
        step: i64,
    },
    Float {
        minimum: Option<f64>,
        maximum: Option<f64>,
        step: f64,
    },
    Fraction,
    Color,
    Option {
        options: Vec<String>,
    },
    Boolean,
    String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LayeredImageItem {
    pub layers: Vec<LayerVisibility>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerVisibility {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub path: String,
    #[serde(default)]
    pub visibility: Option<TimelineValue<TimelineBool>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeItem {
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub shape: TimelineValue<ShapeKind>,
    pub size: TimelineValue<glam::Vec2>,
    #[serde(default = "default_star_points")]
    pub star_points: TimelineValue<u32>,
    #[serde(default = "default_star_inner_radius_percent")]
    pub star_inner_radius_percent: TimelineValue<f32>,
    #[serde(default = "default_arrow_shaft_width_percent")]
    pub arrow_shaft_width_percent: TimelineValue<f32>,
    #[serde(default = "default_arrow_head_length_percent")]
    pub arrow_head_length_percent: TimelineValue<f32>,
    #[serde(default = "default_cross_arm_thickness_percent")]
    pub cross_arm_thickness_percent: TimelineValue<f32>,
    #[serde(default, alias = "fan_inner_radius_percent")]
    pub ellipse_inner_radius_percent: TimelineValue<f32>,
    #[serde(default = "default_ellipse_completion_degrees")]
    pub ellipse_completion_degrees: TimelineValue<f32>,
    pub fill: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub outline_color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub outline_width: TimelineValue<f32>,
    #[serde(default)]
    pub corner_radius: TimelineValue<f32>,
    #[serde(default, deserialize_with = "deserialize_timeline_value")]
    pub rounding_strategy: TimelineValue<ShapeRoundingStrategy>,
    pub shadow_color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub shadow_distance: TimelineValue<f32>,
    #[serde(default = "default_shadow_direction_degrees")]
    pub shadow_direction_degrees: TimelineValue<f32>,
    pub shadow_width: TimelineValue<f32>,
    pub shadow_blur: TimelineValue<f32>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    Triangle,
    Star,
    Arrow,
    Diamond,
    Pentagon,
    Hexagon,
    Heart,
    Octagon,
    Cross,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeRoundingStrategy {
    #[default]
    Continuous,
    Circular,
    Chamfer,
}

shrimply_property_model::timeline_value::timeline_step_type!(
    ShapeKind,
    ShapeKind::Rect,
    &[
        TimelineStepVariant {
            value: ShapeKind::Rect,
            key: "rect",
            label: "Rect",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Ellipse,
            key: "ellipse",
            label: "Ellipse",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Triangle,
            key: "triangle",
            label: "Triangle",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Star,
            key: "star",
            label: "Star",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Arrow,
            key: "arrow",
            label: "Arrow",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Diamond,
            key: "diamond",
            label: "Diamond",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Pentagon,
            key: "pentagon",
            label: "Pentagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Hexagon,
            key: "hexagon",
            label: "Hexagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Heart,
            key: "heart",
            label: "Heart",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Octagon,
            key: "octagon",
            label: "Octagon",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeKind::Cross,
            key: "cross",
            label: "Cross",
            icon: None
        },
    ]
);
shrimply_property_model::timeline_value::timeline_step_type!(
    ShapeRoundingStrategy,
    ShapeRoundingStrategy::Continuous,
    &[
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Continuous,
            key: "continuous",
            label: "Continuous",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Circular,
            key: "circular",
            label: "Circular",
            icon: None
        },
        TimelineStepVariant {
            value: ShapeRoundingStrategy::Chamfer,
            key: "chamfer",
            label: "Chamfer",
            icon: None
        },
    ]
);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextItem {
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub text: TimelineValue<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_families: Vec<FontFamily>,
    #[serde(default, deserialize_with = "deserialize_timeline_value")]
    pub font_style: TimelineValue<TextFontStyle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_variations: Vec<FontVariation>,
    #[serde(default = "default_text_font_weight")]
    pub font_weight: TimelineValue<f32>,
    #[serde(default)]
    pub tracking: TimelineValue<f32>,
    #[serde(default = "default_line_height")]
    pub line_height: TimelineValue<f32>,
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub h_align: TimelineValue<TextHorizontalAlign>,
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub v_align: TimelineValue<VerticalAlign>,
    #[serde(deserialize_with = "deserialize_timeline_value")]
    pub direction: TimelineValue<TextDirection>,
    pub font_size: TimelineValue<f32>,
    pub color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default = "default_text_background_color")]
    pub background_color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub background_roundness: TimelineValue<f32>,
    #[serde(default)]
    pub background_padding: TimelineValue<glam::Vec2>,
    #[serde(default)]
    pub outline_color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub outline_width: TimelineValue<f32>,
    #[serde(default)]
    pub shadow_color: TimelineValue<shrimply_property_model::Color<u8>>,
    #[serde(default)]
    pub shadow_distance: TimelineValue<f32>,
    #[serde(default = "default_shadow_direction_degrees")]
    pub shadow_direction_degrees: TimelineValue<f32>,
    #[serde(default)]
    pub shadow_width: TimelineValue<f32>,
    #[serde(default)]
    pub shadow_blur: TimelineValue<f32>,
}

fn default_text_font_weight() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(400.0)
}

fn default_line_height() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(1.0)
}

fn default_text_background_color() -> TimelineValue<Color<u8>> {
    TimelineValue::new_const(Color::<u8>::from_rgba(0, 0, 0, 0))
}

fn default_shadow_direction_degrees() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(90.0)
}

fn default_star_points() -> TimelineValue<u32> {
    TimelineValue::<u32>::new_const(5)
}

fn default_star_inner_radius_percent() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(45.0)
}

fn default_arrow_shaft_width_percent() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(40.0)
}

fn default_arrow_head_length_percent() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(40.0)
}

fn default_cross_arm_thickness_percent() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(33.0)
}

fn default_ellipse_completion_degrees() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(360.0)
}

impl VisualItem {
    pub fn video_generation_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let mut item = Self::shape_item(canvas_size, start, end);
        item.transform = Transform::fill(canvas_size);
        item.modifiers.clear();
        item.source_width = canvas_size.width.max(1);
        item.source_height = canvas_size.height.max(1);
        item.content = VisualSource::Media;
        item.video_generation = Some(Box::default());
        item
    }

    pub fn paint_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let mut item = Self::shape_item(canvas_size, start, end);
        item.transform = Transform::fill(canvas_size);
        item.modifiers.clear();
        item.source_width = canvas_size.width.max(1);
        item.source_height = canvas_size.height.max(1);
        item.content = VisualSource::Paint(Box::new(PaintItem::new(canvas_size)));
        item
    }

    pub fn background_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let mut item = Self::shape_item(canvas_size, start, end);
        item.transform = Transform::fill(canvas_size);
        item.modifiers.clear();
        item.source_width = canvas_size.width.max(1);
        item.source_height = canvas_size.height.max(1);
        item.content = VisualSource::Background(Box::default());
        item
    }

    pub fn obj_scene_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let mut item = Self::shape_item(canvas_size, start, end);
        item.content = VisualSource::Obj(Box::default());
        item
    }

    pub fn text_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        Self {
            id: Uuid::new_v4(),
            start,
            end,
            time_offset: Time::ZERO,
            source_duration: Time::ZERO,
            playback_speed: default_playback_speed(),
            playback_fps: super::native_playback_fps(),
            repeat_strategy: RepeatStrategy::Hold,
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio: super::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                super::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                super::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                super::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: super::default_mesh_flow_rows(),
            mesh_flow_columns: super::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius: super::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: super::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: Transform::from_resolved(ResolvedTransform {
                position: glam::Vec2::new(width as f32 * 0.5, height as f32 * 0.5),
                anchor: glam::Vec2::ZERO,
                ..ResolvedTransform::IDENTITY
            }),
            modifiers: Vec::new(),
            sample_method: Default::default(),
            skia_drawing_strategy: Default::default(),
            compositing: VisualCompositing::default(),
            visibility: TimelineValue::new_const(TimelineBool::True),
            alpha_mask_video: None,
            transitions: Default::default(),
            svg_color_overrides: Vec::new(),
            source_width: 0,
            source_height: 0,
            default_transform: None,
            group_id: None,
            render_canvas_size: None,
            content: VisualSource::Text(Box::new(TextItem {
                text: TimelineValue::new_const("Text".to_string()),
                font_families: vec![FontFamily::GoogleFonts {
                    name: DEFAULT_TEXT_FONT_FAMILY.to_string(),
                }],
                font_style: TimelineValue::new_const(TextFontStyle::Normal),
                font_variations: Vec::new(),
                font_weight: default_text_font_weight(),
                tracking: TimelineValue::new_const(0.0),
                line_height: default_line_height(),
                h_align: TimelineValue::new_const(TextHorizontalAlign::Center),
                v_align: TimelineValue::new_const(VerticalAlign::Middle),
                direction: TimelineValue::new_const(TextDirection::Horizontal),
                font_size: TimelineValue::<f32>::new_const(64.0),
                color: TimelineValue::<Color<u8>>::new_const(Color::<u8>::WHITE),
                background_color: default_text_background_color(),
                background_roundness: TimelineValue::<f32>::new_const(0.0),
                background_padding: TimelineValue::<glam::Vec2>::new_const(glam::Vec2::ZERO),
                outline_color: TimelineValue::<Color<u8>>::new_const(Color::<u8>::BLACK),
                outline_width: TimelineValue::<f32>::new_const(0.0),
                shadow_color: TimelineValue::<Color<u8>>::new_const(Color::<u8>::BLACK),
                shadow_distance: TimelineValue::<f32>::new_const(0.0),
                shadow_direction_degrees: default_shadow_direction_degrees(),
                shadow_width: TimelineValue::<f32>::new_const(0.0),
                shadow_blur: TimelineValue::<f32>::new_const(0.0),
            })),
            video_generation: None,
            track_id: 0,
            file: Asset::default(),
        }
    }

    pub fn shape_item(canvas_size: CanvasSize, start: Time, end: Time) -> Self {
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        let shape_size = glam::Vec2::splat(300.0);
        Self {
            id: Uuid::new_v4(),
            start,
            end,
            time_offset: Time::ZERO,
            source_duration: Time::ZERO,
            playback_speed: default_playback_speed(),
            playback_fps: super::native_playback_fps(),
            repeat_strategy: RepeatStrategy::Hold,
            stabilize_video: false,
            stabilization_method: Default::default(),
            stabilization_crop_ratio: super::default_video_stabilization_crop_ratio(),
            stabilization_first_derivative_weight:
                super::default_video_stabilization_first_derivative_weight(),
            stabilization_second_derivative_weight:
                super::default_video_stabilization_second_derivative_weight(),
            stabilization_third_derivative_weight:
                super::default_video_stabilization_third_derivative_weight(),
            mesh_flow_rows: super::default_mesh_flow_rows(),
            mesh_flow_columns: super::default_mesh_flow_columns(),
            mesh_flow_smoothing_radius: super::default_mesh_flow_smoothing_radius(),
            mesh_flow_iterations: super::default_mesh_flow_iterations(),
            mesh_flow_adaptive_weights: Default::default(),
            animation_time_offset: Time::ZERO,
            motion_blur: Default::default(),
            transform: Transform::from_resolved(ResolvedTransform {
                position: glam::Vec2::new(width as f32 * 0.5, height as f32 * 0.5),
                anchor: shape_size * 0.5,
                ..ResolvedTransform::IDENTITY
            }),
            modifiers: Vec::new(),
            sample_method: Default::default(),
            skia_drawing_strategy: Default::default(),
            compositing: VisualCompositing::default(),
            visibility: TimelineValue::new_const(TimelineBool::True),
            alpha_mask_video: None,
            transitions: Default::default(),
            svg_color_overrides: Vec::new(),
            source_width: 0,
            source_height: 0,
            default_transform: None,
            group_id: None,
            render_canvas_size: None,
            content: VisualSource::Shape(Box::new(ShapeItem {
                shape: TimelineValue::new_const(ShapeKind::Rect),
                size: TimelineValue::<glam::Vec2>::new_const(shape_size),
                star_points: default_star_points(),
                star_inner_radius_percent: default_star_inner_radius_percent(),
                arrow_shaft_width_percent: default_arrow_shaft_width_percent(),
                arrow_head_length_percent: default_arrow_head_length_percent(),
                cross_arm_thickness_percent: default_cross_arm_thickness_percent(),
                ellipse_inner_radius_percent: TimelineValue::<f32>::new_const(0.0),
                ellipse_completion_degrees: default_ellipse_completion_degrees(),
                fill: TimelineValue::<Color<u8>>::new_const(Color::<u8>::from_rgb(53, 132, 228)),
                outline_color: TimelineValue::<Color<u8>>::new_const(Color::<u8>::BLACK),
                outline_width: TimelineValue::<f32>::new_const(0.0),
                corner_radius: TimelineValue::<f32>::new_const(0.0),
                rounding_strategy: TimelineValue::new_const(ShapeRoundingStrategy::Continuous),
                shadow_color: TimelineValue::<Color<u8>>::new_const(Color::<u8>::BLACK),
                shadow_distance: TimelineValue::<f32>::new_const(0.0),
                shadow_direction_degrees: default_shadow_direction_degrees(),
                shadow_width: TimelineValue::<f32>::new_const(0.0),
                shadow_blur: TimelineValue::<f32>::new_const(0.0),
            })),
            video_generation: None,
            track_id: 0,
            file: Asset::default(),
        }
    }

    pub fn is_media(&self) -> bool {
        matches!(
            self.content,
            VisualSource::Media
                | VisualSource::Image
                | VisualSource::Gif
                | VisualSource::Svg
                | VisualSource::Pdf(_)
                | VisualSource::Blender(_)
                | VisualSource::LayeredImage(_)
                | VisualSource::Obj(_)
                | VisualSource::Gaussian(_)
        )
    }

    pub fn uses_file_asset(&self) -> bool {
        matches!(
            self.content,
            VisualSource::Media
                | VisualSource::Image
                | VisualSource::Gif
                | VisualSource::Svg
                | VisualSource::Pdf(_)
                | VisualSource::Manim(_)
                | VisualSource::Blender(_)
                | VisualSource::LayeredImage(_)
                | VisualSource::Obj(_)
                | VisualSource::Gaussian(_)
        )
    }

    pub fn is_video_media(&self) -> bool {
        matches!(self.content, VisualSource::Media)
    }

    pub fn is_static_visual_media(&self) -> bool {
        matches!(
            self.content,
            VisualSource::Image
                | VisualSource::Svg
                | VisualSource::Pdf(_)
                | VisualSource::LayeredImage(_)
                | VisualSource::Obj(_)
                | VisualSource::Gaussian(_)
        )
    }

    pub fn is_generated(&self) -> bool {
        matches!(
            self.content,
            VisualSource::Text(_)
                | VisualSource::Shape(_)
                | VisualSource::Paint(_)
                | VisualSource::Background(_)
        )
    }

    pub fn supports_vector_transitions(&self) -> bool {
        matches!(
            self.content,
            VisualSource::Svg | VisualSource::Text(_) | VisualSource::Shape(_)
        )
    }

    pub fn repeats_keyframes(&self) -> bool {
        self.is_generated() || self.is_static_visual_media()
    }
}
