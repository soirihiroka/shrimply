mod glb;
mod obj;

use glam::Vec3;
use serde::{Deserialize, Serialize};
use shrimply_asset::Asset;
use shrimply_property_model::Color;
use shrimply_property_model::timeline_value::*;
pub use shrimply_transform_3d::{
    COLMAP_TRACKING_MODEL, CameraSource, ColmapCameraModel, ColmapQuality, MAX_EXPOSURE_EV,
    MAX_F_STOP, MIN_EXPOSURE_EV, MIN_F_STOP, Projection as CameraProjection,
    ResolvedTransform3D as ResolvedTransform3d, RotationOrder, TrackingCameraSource,
    TrackingSettings, Transform3D, VGGT_SLAM_TRACKING_MODEL, focal_length_mm, vertical_fov_degrees,
};

pub use glb::{GlbError, load_glb};
pub use obj::{MeshMaterial, ObjError, ObjMesh, TextureAtlas, TextureMapping, load_obj, parse_obj};

pub const DEFAULT_CAMERA_VERTICAL_FOV_DEGREES: f32 = 50.0;
pub const DEFAULT_CAMERA_FRAMING_MARGIN: f32 = 1.1;
pub const DEFAULT_MODEL_POSITION: Vec3 = Vec3::ZERO;
pub const DEFAULT_MODEL_ANCHOR: Vec3 = Vec3::ZERO;
pub const DEFAULT_MODEL_ROTATION_DEGREES: Vec3 = Vec3::ZERO;
pub const DEFAULT_MODEL_SCALE: Vec3 = Vec3::ONE;
pub const DEFAULT_CAMERA_ROTATION_DEGREES: Vec3 = Vec3::ZERO;
pub const DEFAULT_ORTHOGRAPHIC_HEIGHT: f32 = 2.2;
pub const DEFAULT_FOCUS_DISTANCE: f32 = 0.0;
pub const DEFAULT_BACKGROUND_DISTANCE: f32 = 10.0;
pub const DEFAULT_COMPOSED_PLANE_INTENSITY: f32 = 1.0;
pub const DEFAULT_F_STOP: f32 = 2.8;
pub const DEFAULT_EXPOSURE_EV: f32 = 0.0;
pub const DEFAULT_MATERIAL_BASE_COLOR: Color<u8> = Color::<u8>::from_rgb(0xc8, 0xc8, 0xc8);
pub const DEFAULT_METALLIC: f32 = 0.0;
pub const DEFAULT_ROUGHNESS: f32 = 0.5;
pub const DEFAULT_SUBSURFACE: f32 = 0.0;
pub const DEFAULT_CLEARCOAT: f32 = 0.0;
pub const DEFAULT_SHEEN: f32 = 0.0;
pub const DEFAULT_TRANSMISSION: f32 = 0.0;
pub const DEFAULT_IOR: f32 = 1.5;
pub const DEFAULT_TOON_BANDS: f32 = 4.0;
pub const DEFAULT_TOON_COLOR_LEVELS: f32 = 8.0;
pub const DEFAULT_TOON_KUWAHARA_RADIUS: f32 = 2.0;
pub const DEFAULT_TOON_KUWAHARA_STRENGTH: f32 = 1.0;
pub const DEFAULT_TOON_SHADOW_COLOR: Color<u8> = Color::<u8>::from_rgb(0x22, 0x1b, 0x3d);
pub const DEFAULT_TOON_SHADOW_STRENGTH: f32 = 0.65;
pub const DEFAULT_TOON_SHADOW_DARKEST_TONE: f32 = 0.35;
pub const MIN_TOON_SHADOW_TONE: f32 = 0.05;
pub const DEFAULT_TOON_SHADOW_DOT_SIZE: f32 = 3.0;
pub const DEFAULT_TOON_SHADOW_DOT_DENSITY: f32 = 0.15;
pub const DEFAULT_TOON_SHADOW_DOT_DISTRIBUTION_RANDOMNESS: f32 = 0.0;
pub const DEFAULT_TOON_SHADOW_DOT_SIZE_RANDOMNESS: f32 = 0.0;
pub const DEFAULT_TOON_SHADOW_LINE_DIRECTION_DEGREES: f32 = 45.0;
pub const DEFAULT_TOON_SHADOW_LINE_WIDTH: f32 = 1.5;
pub const DEFAULT_TOON_SHADOW_LINE_DENSITY: f32 = 0.15;
pub const DEFAULT_TOON_SHADOW_LINE_DISTRIBUTION_RANDOMNESS: f32 = 0.0;
pub const DEFAULT_TOON_SHADOW_LINE_WIDTH_RANDOMNESS: f32 = 0.0;
pub const DEFAULT_TOON_SHADOW_PATTERN_SOFTNESS: f32 = 0.5;
pub const DEFAULT_TOON_SHADOW_CROSSHATCH_ANGLE_DEGREES: f32 = 90.0;
pub const DEFAULT_TOON_SHADOW_CROSSHATCH_MAX_DIRECTIONS: f32 = 4.0;
pub const DEFAULT_TOON_RIM_COLOR: Color<u8> = Color::<u8>::from_rgb(0x66, 0xe3, 0xff);
pub const DEFAULT_TOON_RIM_STRENGTH: f32 = 0.8;
pub const DEFAULT_TOON_RIM_POWER: f32 = 3.0;
pub const DEFAULT_TOON_SPECULAR_SIZE: f32 = 0.12;
pub const DEFAULT_TOON_SPECULAR_STRENGTH: f32 = 0.8;
pub const DEFAULT_TOON_OUTLINE_COLOR: Color<u8> = Color::<u8>::from_rgb(0x08, 0x09, 0x12);
pub const DEFAULT_TOON_OUTLINE_WIDTH: f32 = 2.0;
pub const DEFAULT_TOON_OUTLINE_OPACITY: f32 = 1.0;
pub const DEFAULT_TOON_OUTLINE_DEPTH_THRESHOLD: f32 = 0.02;
pub const DEFAULT_TOON_OUTLINE_NORMAL_ANGLE_DEGREES: f32 = 35.0;
pub const DEFAULT_TOON_OUTLINE_DOG_INNER_RADIUS: f32 = 1.0;
pub const DEFAULT_TOON_OUTLINE_DOG_RADIUS_RATIO: f32 = 1.6;
pub const DEFAULT_TOON_OUTLINE_DOG_THRESHOLD: f32 = 0.01;
pub const DEFAULT_TOON_OUTLINE_DOG_SHARPNESS: f32 = 16.0;
pub const DEFAULT_TOON_OUTLINE_OFFSET_VARIATION: f32 = 0.0;
pub const DEFAULT_TOON_OUTLINE_WIDTH_VARIATION: f32 = 0.0;
pub const DEFAULT_TOON_OUTLINE_OFFSET_FREQUENCY: f32 = 0.1;
pub const DEFAULT_TOON_OUTLINE_WIDTH_FREQUENCY: f32 = 0.1;
pub const DEFAULT_TOON_OUTLINE_AGGRESSIVENESS: f32 = 1.0;
pub const DEFAULT_TOON_OUTLINE_NOISE_SEED: f32 = 0.0;
pub const DEFAULT_TOON_OUTLINE_NOISE_EVOLUTION: f32 = 0.0;
pub const DEFAULT_ENVIRONMENT_ROTATION_DEGREES: Vec3 = Vec3::ZERO;
pub const DEFAULT_ENVIRONMENT_INTENSITY: f32 = 1.0;
pub const DEFAULT_SHADOW_RECEIVER_POSITION: Vec3 = Vec3::new(0.0, -1.0, 0.0);
pub const DEFAULT_SHADOW_RECEIVER_OPACITY: f32 = 1.0;
pub const MIN_VERTICAL_FOV_DEGREES: f32 = 1.0;
pub const MAX_VERTICAL_FOV_DEGREES: f32 = 179.0;
pub const MIN_ORTHOGRAPHIC_HEIGHT: f32 = 0.001;
pub const MIN_ROUGHNESS: f32 = 0.0;
pub const MIN_IOR: f32 = 0.5;
pub const MAX_IOR: f32 = 3.0;
pub const MIN_POINT_LIGHT_RANGE: f32 = 0.001;
pub const MIN_NEAR_PLANE: f32 = 0.001;

pub fn default_camera_distance() -> f32 {
    DEFAULT_CAMERA_FRAMING_MARGIN
        / (DEFAULT_CAMERA_VERTICAL_FOV_DEGREES * 0.5)
            .to_radians()
            .sin()
}

pub type AnimatedVec3 = TimelineValue<Vec3>;
pub type Transform3d = Transform3D<AnimatedVec3, TimelineValue<RotationOrder>>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiAliasing {
    #[default]
    None,
    RotatedGrid2x,
    Grid4x,
    Stochastic8x,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundAddressMode {
    ExtendEdge,
    #[default]
    Repeat,
    Mirror,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera3d {
    #[serde(default)]
    pub source: CameraSource,
    pub projection: CameraProjection,
    #[serde(default)]
    pub anti_aliasing: AntiAliasing,
    pub position: AnimatedVec3,
    pub rotation_degrees: AnimatedVec3,
    pub vertical_fov_degrees: TimelineValue<f32>,
    pub orthographic_height: TimelineValue<f32>,
    #[serde(default = "default_focus_distance")]
    pub focus_distance: TimelineValue<f32>,
    #[serde(default = "default_background_distance")]
    pub background_distance: TimelineValue<f32>,
    #[serde(default = "default_true")]
    pub background_plane_enabled: bool,
    #[serde(default = "default_composed_plane_intensity")]
    pub background_intensity: TimelineValue<f32>,
    #[serde(default)]
    pub background_address_mode: BackgroundAddressMode,
    #[serde(default = "default_f_stop")]
    pub f_stop: TimelineValue<f32>,
    pub exposure_ev: TimelineValue<f32>,
}

impl Default for Camera3d {
    fn default() -> Self {
        Self {
            source: CameraSource::Custom,
            projection: CameraProjection::Perspective,
            anti_aliasing: AntiAliasing::RotatedGrid2x,
            position: AnimatedVec3::new_const(Vec3::new(0.0, 0.0, default_camera_distance())),
            rotation_degrees: AnimatedVec3::new_const(DEFAULT_CAMERA_ROTATION_DEGREES),
            vertical_fov_degrees: TimelineValue::<f32>::new_const(
                DEFAULT_CAMERA_VERTICAL_FOV_DEGREES,
            ),
            orthographic_height: TimelineValue::<f32>::new_const(DEFAULT_ORTHOGRAPHIC_HEIGHT),
            focus_distance: default_focus_distance(),
            background_distance: default_background_distance(),
            background_plane_enabled: true,
            background_intensity: default_composed_plane_intensity(),
            background_address_mode: BackgroundAddressMode::default(),
            f_stop: default_f_stop(),
            exposure_ev: TimelineValue::<f32>::new_const(DEFAULT_EXPOSURE_EV),
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum NormalMode {
    #[default]
    #[strum(to_string = "Phong")]
    Smooth,
    #[strum(to_string = "SLERP")]
    Spherical,
    #[strum(to_string = "PN triangles")]
    PnTriangle,
    Flat,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
)]
#[serde(rename_all = "snake_case")]
pub enum ShadingModel {
    #[default]
    #[strum(to_string = "PBR")]
    Pbr,
    Toon,
    Depth,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathTracingMode {
    #[default]
    Off,
    Samples1,
    Samples2,
    Preview,
    Samples8,
    Quality,
    Samples32,
    Samples64,
    Samples128,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightSamplingQuality {
    Rays1,
    Rays2,
    Standard,
    #[default]
    High,
    Ultra,
    Rays32,
    Rays64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonOutlineMode {
    #[default]
    Off,
    Silhouette,
    SilhouetteAndCreases,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonOutlineQuality {
    Standard,
    #[default]
    High,
    Ultra,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonOutlineMethod {
    RayTraced,
    Fresnel,
    Hybrid,
    Sobel,
    RobertsCross,
    DifferenceOfGaussians,
    #[default]
    RegionBoundary,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonTextureFilter {
    Direct,
    #[default]
    Kuwahara,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToonShadowKind {
    #[default]
    Solid,
    Dots,
    Lines,
    Crosshatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToonOutline {
    pub mode: ToonOutlineMode,
    #[serde(default)]
    pub method: ToonOutlineMethod,
    pub quality: ToonOutlineQuality,
    pub color: TimelineValue<shrimply_property_model::Color<u8>>,
    pub width: TimelineValue<f32>,
    pub opacity: TimelineValue<f32>,
    pub depth_threshold: TimelineValue<f32>,
    pub normal_angle_degrees: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_dog_inner_radius")]
    pub dog_inner_radius: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_dog_radius_ratio")]
    pub dog_radius_ratio: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_dog_threshold")]
    pub dog_threshold: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_dog_sharpness")]
    pub dog_sharpness: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_offset_variation")]
    pub offset_variation: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_width_variation")]
    pub width_variation: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_offset_frequency")]
    pub offset_frequency: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_width_frequency")]
    pub width_frequency: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_aggressiveness")]
    pub aggressiveness: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_noise_seed")]
    pub noise_seed: TimelineValue<f32>,
    #[serde(default = "default_toon_outline_noise_evolution")]
    pub noise_evolution: TimelineValue<f32>,
}

impl Default for ToonOutline {
    fn default() -> Self {
        Self {
            mode: ToonOutlineMode::Off,
            method: ToonOutlineMethod::RegionBoundary,
            quality: ToonOutlineQuality::High,
            color: TimelineValue::<Color<u8>>::new_const(DEFAULT_TOON_OUTLINE_COLOR),
            width: TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_WIDTH),
            opacity: TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_OPACITY),
            depth_threshold: TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_DEPTH_THRESHOLD),
            normal_angle_degrees: TimelineValue::<f32>::new_const(
                DEFAULT_TOON_OUTLINE_NORMAL_ANGLE_DEGREES,
            ),
            dog_inner_radius: default_toon_outline_dog_inner_radius(),
            dog_radius_ratio: default_toon_outline_dog_radius_ratio(),
            dog_threshold: default_toon_outline_dog_threshold(),
            dog_sharpness: default_toon_outline_dog_sharpness(),
            offset_variation: default_toon_outline_offset_variation(),
            width_variation: default_toon_outline_width_variation(),
            offset_frequency: default_toon_outline_offset_frequency(),
            width_frequency: default_toon_outline_width_frequency(),
            aggressiveness: default_toon_outline_aggressiveness(),
            noise_seed: default_toon_outline_noise_seed(),
            noise_evolution: default_toon_outline_noise_evolution(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToonMaterial {
    pub bands: TimelineValue<f32>,
    #[serde(default)]
    pub texture_filter: ToonTextureFilter,
    #[serde(default = "default_toon_color_levels")]
    pub color_levels: TimelineValue<f32>,
    #[serde(default = "default_toon_kuwahara_radius")]
    pub kuwahara_radius: TimelineValue<f32>,
    #[serde(default = "default_toon_kuwahara_strength")]
    pub kuwahara_strength: TimelineValue<f32>,
    #[serde(default)]
    pub shadow_kind: ToonShadowKind,
    pub shadow_color: TimelineValue<shrimply_property_model::Color<u8>>,
    pub shadow_strength: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_darkest_tone")]
    pub shadow_darkest_tone: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_dot_size")]
    pub shadow_dot_size: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_dot_density")]
    pub shadow_dot_density: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_dot_distribution_randomness")]
    pub shadow_dot_distribution_randomness: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_dot_size_randomness")]
    pub shadow_dot_size_randomness: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_line_direction_degrees")]
    pub shadow_line_direction_degrees: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_line_width")]
    pub shadow_line_width: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_line_density")]
    pub shadow_line_density: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_line_distribution_randomness")]
    pub shadow_line_distribution_randomness: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_line_width_randomness")]
    pub shadow_line_width_randomness: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_pattern_softness")]
    pub shadow_pattern_softness: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_crosshatch_angle_degrees")]
    pub shadow_crosshatch_angle_degrees: TimelineValue<f32>,
    #[serde(default = "default_toon_shadow_crosshatch_max_directions")]
    pub shadow_crosshatch_max_directions: TimelineValue<f32>,
    pub rim_color: TimelineValue<shrimply_property_model::Color<u8>>,
    pub rim_strength: TimelineValue<f32>,
    pub rim_power: TimelineValue<f32>,
    pub specular_size: TimelineValue<f32>,
    pub specular_strength: TimelineValue<f32>,
    #[serde(default)]
    pub outline: ToonOutline,
}

impl Default for ToonMaterial {
    fn default() -> Self {
        Self {
            bands: TimelineValue::<f32>::new_const(DEFAULT_TOON_BANDS),
            texture_filter: ToonTextureFilter::Kuwahara,
            color_levels: default_toon_color_levels(),
            kuwahara_radius: default_toon_kuwahara_radius(),
            kuwahara_strength: default_toon_kuwahara_strength(),
            shadow_kind: ToonShadowKind::Solid,
            shadow_color: TimelineValue::<Color<u8>>::new_const(DEFAULT_TOON_SHADOW_COLOR),
            shadow_strength: TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_STRENGTH),
            shadow_darkest_tone: default_toon_shadow_darkest_tone(),
            shadow_dot_size: default_toon_shadow_dot_size(),
            shadow_dot_density: default_toon_shadow_dot_density(),
            shadow_dot_distribution_randomness: default_toon_shadow_dot_distribution_randomness(),
            shadow_dot_size_randomness: default_toon_shadow_dot_size_randomness(),
            shadow_line_direction_degrees: default_toon_shadow_line_direction_degrees(),
            shadow_line_width: default_toon_shadow_line_width(),
            shadow_line_density: default_toon_shadow_line_density(),
            shadow_line_distribution_randomness: default_toon_shadow_line_distribution_randomness(),
            shadow_line_width_randomness: default_toon_shadow_line_width_randomness(),
            shadow_pattern_softness: default_toon_shadow_pattern_softness(),
            shadow_crosshatch_angle_degrees: default_toon_shadow_crosshatch_angle_degrees(),
            shadow_crosshatch_max_directions: default_toon_shadow_crosshatch_max_directions(),
            rim_color: TimelineValue::<Color<u8>>::new_const(DEFAULT_TOON_RIM_COLOR),
            rim_strength: TimelineValue::<f32>::new_const(DEFAULT_TOON_RIM_STRENGTH),
            rim_power: TimelineValue::<f32>::new_const(DEFAULT_TOON_RIM_POWER),
            specular_size: TimelineValue::<f32>::new_const(DEFAULT_TOON_SPECULAR_SIZE),
            specular_strength: TimelineValue::<f32>::new_const(DEFAULT_TOON_SPECULAR_STRENGTH),
            outline: ToonOutline::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PbrMaterial {
    pub base_color: TimelineValue<shrimply_property_model::Color<u8>>,
    pub metallic: TimelineValue<f32>,
    pub roughness: TimelineValue<f32>,
    #[serde(default = "default_subsurface")]
    pub subsurface: TimelineValue<f32>,
    #[serde(default = "default_clearcoat")]
    pub clearcoat: TimelineValue<f32>,
    #[serde(default = "default_sheen")]
    pub sheen: TimelineValue<f32>,
    #[serde(default = "default_transmission")]
    pub transmission: TimelineValue<f32>,
    #[serde(default = "default_ior")]
    pub ior: TimelineValue<f32>,
    #[serde(default)]
    pub path_tracing: PathTracingMode,
    #[serde(default)]
    pub light_sampling_quality: LightSamplingQuality,
    #[serde(
        default,
        alias = "optix_denoising",
        deserialize_with = "deserialize_denoising"
    )]
    pub denoising: DenoisingMethod,
    pub normal_mode: NormalMode,
    #[serde(default)]
    pub shading_model: ShadingModel,
    #[serde(default)]
    pub toon: ToonMaterial,
}

pub fn material_numbers(material: &PbrMaterial) -> Vec<&TimelineValue<f32>> {
    vec![
        &material.metallic,
        &material.roughness,
        &material.subsurface,
        &material.clearcoat,
        &material.sheen,
        &material.transmission,
        &material.ior,
        &material.toon.bands,
        &material.toon.color_levels,
        &material.toon.kuwahara_radius,
        &material.toon.kuwahara_strength,
        &material.toon.shadow_strength,
        &material.toon.shadow_darkest_tone,
        &material.toon.shadow_dot_size,
        &material.toon.shadow_dot_density,
        &material.toon.shadow_dot_distribution_randomness,
        &material.toon.shadow_dot_size_randomness,
        &material.toon.shadow_line_direction_degrees,
        &material.toon.shadow_line_width,
        &material.toon.shadow_line_density,
        &material.toon.shadow_line_distribution_randomness,
        &material.toon.shadow_line_width_randomness,
        &material.toon.shadow_pattern_softness,
        &material.toon.shadow_crosshatch_angle_degrees,
        &material.toon.shadow_crosshatch_max_directions,
        &material.toon.rim_strength,
        &material.toon.rim_power,
        &material.toon.specular_size,
        &material.toon.specular_strength,
        &material.toon.outline.width,
        &material.toon.outline.opacity,
        &material.toon.outline.depth_threshold,
        &material.toon.outline.normal_angle_degrees,
        &material.toon.outline.dog_inner_radius,
        &material.toon.outline.dog_radius_ratio,
        &material.toon.outline.dog_threshold,
        &material.toon.outline.dog_sharpness,
        &material.toon.outline.offset_variation,
        &material.toon.outline.width_variation,
        &material.toon.outline.offset_frequency,
        &material.toon.outline.width_frequency,
        &material.toon.outline.aggressiveness,
        &material.toon.outline.noise_seed,
        &material.toon.outline.noise_evolution,
    ]
}

pub fn material_numbers_mut(material: &mut PbrMaterial) -> Vec<&mut TimelineValue<f32>> {
    vec![
        &mut material.metallic,
        &mut material.roughness,
        &mut material.subsurface,
        &mut material.clearcoat,
        &mut material.sheen,
        &mut material.transmission,
        &mut material.ior,
        &mut material.toon.bands,
        &mut material.toon.color_levels,
        &mut material.toon.kuwahara_radius,
        &mut material.toon.kuwahara_strength,
        &mut material.toon.shadow_strength,
        &mut material.toon.shadow_darkest_tone,
        &mut material.toon.shadow_dot_size,
        &mut material.toon.shadow_dot_density,
        &mut material.toon.shadow_dot_distribution_randomness,
        &mut material.toon.shadow_dot_size_randomness,
        &mut material.toon.shadow_line_direction_degrees,
        &mut material.toon.shadow_line_width,
        &mut material.toon.shadow_line_density,
        &mut material.toon.shadow_line_distribution_randomness,
        &mut material.toon.shadow_line_width_randomness,
        &mut material.toon.shadow_pattern_softness,
        &mut material.toon.shadow_crosshatch_angle_degrees,
        &mut material.toon.shadow_crosshatch_max_directions,
        &mut material.toon.rim_strength,
        &mut material.toon.rim_power,
        &mut material.toon.specular_size,
        &mut material.toon.specular_strength,
        &mut material.toon.outline.width,
        &mut material.toon.outline.opacity,
        &mut material.toon.outline.depth_threshold,
        &mut material.toon.outline.normal_angle_degrees,
        &mut material.toon.outline.dog_inner_radius,
        &mut material.toon.outline.dog_radius_ratio,
        &mut material.toon.outline.dog_threshold,
        &mut material.toon.outline.dog_sharpness,
        &mut material.toon.outline.offset_variation,
        &mut material.toon.outline.width_variation,
        &mut material.toon.outline.offset_frequency,
        &mut material.toon.outline.width_frequency,
        &mut material.toon.outline.aggressiveness,
        &mut material.toon.outline.noise_seed,
        &mut material.toon.outline.noise_evolution,
    ]
}

pub fn material_colors(
    material: &PbrMaterial,
) -> Vec<&TimelineValue<shrimply_property_model::Color<u8>>> {
    vec![
        &material.base_color,
        &material.toon.shadow_color,
        &material.toon.rim_color,
        &material.toon.outline.color,
    ]
}

pub fn material_colors_mut(
    material: &mut PbrMaterial,
) -> Vec<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
    vec![
        &mut material.base_color,
        &mut material.toon.shadow_color,
        &mut material.toon.rim_color,
        &mut material.toon.outline.color,
    ]
}

impl Default for PbrMaterial {
    fn default() -> Self {
        Self {
            base_color: TimelineValue::<Color<u8>>::new_const(DEFAULT_MATERIAL_BASE_COLOR),
            metallic: TimelineValue::<f32>::new_const(DEFAULT_METALLIC),
            roughness: TimelineValue::<f32>::new_const(DEFAULT_ROUGHNESS),
            subsurface: default_subsurface(),
            clearcoat: default_clearcoat(),
            sheen: default_sheen(),
            transmission: default_transmission(),
            ior: default_ior(),
            path_tracing: PathTracingMode::Off,
            light_sampling_quality: LightSamplingQuality::High,
            denoising: DenoisingMethod::Auto,
            normal_mode: NormalMode::Smooth,
            shading_model: ShadingModel::Pbr,
            toon: ToonMaterial::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShadowReceiverPlane3d {
    pub enabled: TimelineValue<TimelineBool>,
    #[serde(default = "default_true")]
    pub composite_enabled: bool,
    #[serde(default = "default_composed_plane_intensity")]
    pub intensity: TimelineValue<f32>,
    pub position: AnimatedVec3,
    pub rotation_degrees: AnimatedVec3,
    pub opacity: TimelineValue<f32>,
    #[serde(default = "default_ground_shadow_strength")]
    pub shadow_strength: TimelineValue<f32>,
    #[serde(default = "default_ground_reflection")]
    pub reflection: TimelineValue<f32>,
    #[serde(default = "default_ground_roughness")]
    pub roughness: TimelineValue<f32>,
}

const DEFAULT_GROUND_SHADOW_STRENGTH: f32 = 1.0;
const DEFAULT_GROUND_REFLECTION: f32 = 0.0;
const DEFAULT_GROUND_ROUGHNESS: f32 = 0.0;

fn default_ground_shadow_strength() -> TimelineValue<f32> {
    TimelineValue::new_const(DEFAULT_GROUND_SHADOW_STRENGTH)
}

fn default_ground_reflection() -> TimelineValue<f32> {
    TimelineValue::new_const(DEFAULT_GROUND_REFLECTION)
}

fn default_ground_roughness() -> TimelineValue<f32> {
    TimelineValue::new_const(DEFAULT_GROUND_ROUGHNESS)
}

impl Default for ShadowReceiverPlane3d {
    fn default() -> Self {
        Self {
            enabled: TimelineValue::new_const(TimelineBool::False),
            composite_enabled: true,
            intensity: default_composed_plane_intensity(),
            position: AnimatedVec3::new_const(DEFAULT_SHADOW_RECEIVER_POSITION),
            rotation_degrees: AnimatedVec3::new_const(Vec3::ZERO),
            opacity: TimelineValue::<f32>::new_const(DEFAULT_SHADOW_RECEIVER_OPACITY),
            shadow_strength: default_ground_shadow_strength(),
            reflection: default_ground_reflection(),
            roughness: default_ground_roughness(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentSource {
    Composite,
    Image,
    Black,
}

fn default_environment_color() -> TimelineValue<Color<u8>> {
    TimelineValue::new_const(Color::<u8>::BLACK)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment3d {
    #[serde(default)]
    pub source: Option<EnvironmentSource>,
    pub file: Option<Asset>,
    #[serde(default = "default_environment_color")]
    pub solid_color: TimelineValue<Color<u8>>,
    pub rotation_degrees: AnimatedVec3,
    pub intensity: TimelineValue<f32>,
}

impl Environment3d {
    pub fn effective_source(&self) -> EnvironmentSource {
        self.source.unwrap_or(if self.file.is_some() {
            EnvironmentSource::Image
        } else {
            EnvironmentSource::Composite
        })
    }
}

impl Default for Environment3d {
    fn default() -> Self {
        Self {
            source: Some(EnvironmentSource::Composite),
            file: None,
            solid_color: default_environment_color(),
            rotation_degrees: AnimatedVec3::new_const(DEFAULT_ENVIRONMENT_ROTATION_DEGREES),
            intensity: TimelineValue::<f32>::new_const(DEFAULT_ENVIRONMENT_INTENSITY),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ObjScene {
    pub model: Transform3d,
    pub camera: Camera3d,
    pub material: PbrMaterial,
    #[serde(default)]
    pub shadow_receiver: ShadowReceiverPlane3d,
    pub environment: Environment3d,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedCamera3d {
    pub projection: CameraProjection,
    pub anti_aliasing: AntiAliasing,
    pub position: Vec3,
    pub rotation_degrees: Vec3,
    pub vertical_fov_degrees: f32,
    pub orthographic_height: f32,
    pub focus_distance: f32,
    pub background_distance: f32,
    pub background_plane_enabled: bool,
    pub background_intensity: f32,
    pub background_address_mode: BackgroundAddressMode,
    pub f_stop: f32,
    pub exposure_ev: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedPbrMaterial {
    pub base_color: Color<u8>,
    pub metallic: f32,
    pub roughness: f32,
    pub subsurface: f32,
    pub clearcoat: f32,
    pub sheen: f32,
    pub transmission: f32,
    pub ior: f32,
    pub path_tracing: PathTracingMode,
    pub light_sampling_quality: LightSamplingQuality,
    pub denoising: DenoisingMethod,
    pub normal_mode: NormalMode,
    pub shading_model: ShadingModel,
    pub toon: ResolvedToonMaterial,
}

fn default_subsurface() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_SUBSURFACE)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
#[serde(rename_all = "snake_case")]
pub enum DenoisingMethod {
    Off = 0,
    #[default]
    Auto = 1,
    Optix = 2,
    OpenImageDenoise = 3,
    Atrous = 4,
    MetalSvgf = 5,
}

fn deserialize_denoising<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<DenoisingMethod, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
        Legacy(bool),
        Method(DenoisingMethod),
    }
    Ok(match Saved::deserialize(deserializer)? {
        Saved::Legacy(true) => DenoisingMethod::Auto,
        Saved::Legacy(false) => DenoisingMethod::Off,
        Saved::Method(method) => method,
    })
}

fn default_clearcoat() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_CLEARCOAT)
}

fn default_sheen() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_SHEEN)
}

fn default_transmission() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TRANSMISSION)
}

fn default_ior() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_IOR)
}

fn default_focus_distance() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_FOCUS_DISTANCE)
}

fn default_background_distance() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_BACKGROUND_DISTANCE)
}

fn default_composed_plane_intensity() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_COMPOSED_PLANE_INTENSITY)
}

fn default_true() -> bool {
    true
}

fn default_f_stop() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_F_STOP)
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedToonMaterial {
    pub bands: f32,
    pub texture_filter: ToonTextureFilter,
    pub color_levels: f32,
    pub kuwahara_radius: f32,
    pub kuwahara_strength: f32,
    pub shadow_kind: ToonShadowKind,
    pub shadow_color: Color<u8>,
    pub shadow_strength: f32,
    pub shadow_darkest_tone: f32,
    pub shadow_dot_size: f32,
    pub shadow_dot_density: f32,
    pub shadow_dot_distribution_randomness: f32,
    pub shadow_dot_size_randomness: f32,
    pub shadow_line_direction_degrees: f32,
    pub shadow_line_width: f32,
    pub shadow_line_density: f32,
    pub shadow_line_distribution_randomness: f32,
    pub shadow_line_width_randomness: f32,
    pub shadow_pattern_softness: f32,
    pub shadow_crosshatch_angle_degrees: f32,
    pub shadow_crosshatch_max_directions: f32,
    pub rim_color: Color<u8>,
    pub rim_strength: f32,
    pub rim_power: f32,
    pub specular_size: f32,
    pub specular_strength: f32,
    pub outline: ResolvedToonOutline,
}

fn default_toon_color_levels() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_COLOR_LEVELS)
}

fn default_toon_kuwahara_radius() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_KUWAHARA_RADIUS)
}

fn default_toon_kuwahara_strength() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_KUWAHARA_STRENGTH)
}

fn default_toon_shadow_dot_size() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_DOT_SIZE)
}

fn default_toon_shadow_darkest_tone() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_DARKEST_TONE)
}

fn default_toon_shadow_dot_density() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_DOT_DENSITY)
}

fn default_toon_shadow_dot_distribution_randomness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_DOT_DISTRIBUTION_RANDOMNESS)
}

fn default_toon_shadow_dot_size_randomness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_DOT_SIZE_RANDOMNESS)
}

fn default_toon_shadow_line_direction_degrees() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_LINE_DIRECTION_DEGREES)
}

fn default_toon_shadow_line_width() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_LINE_WIDTH)
}

fn default_toon_shadow_line_density() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_LINE_DENSITY)
}

fn default_toon_shadow_line_distribution_randomness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_LINE_DISTRIBUTION_RANDOMNESS)
}

fn default_toon_shadow_line_width_randomness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_LINE_WIDTH_RANDOMNESS)
}

fn default_toon_shadow_pattern_softness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_PATTERN_SOFTNESS)
}

fn default_toon_shadow_crosshatch_angle_degrees() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_CROSSHATCH_ANGLE_DEGREES)
}

fn default_toon_shadow_crosshatch_max_directions() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_SHADOW_CROSSHATCH_MAX_DIRECTIONS)
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedToonOutline {
    pub mode: ToonOutlineMode,
    pub method: ToonOutlineMethod,
    pub quality: ToonOutlineQuality,
    pub color: Color<u8>,
    pub width: f32,
    pub opacity: f32,
    pub depth_threshold: f32,
    pub normal_angle_degrees: f32,
    pub dog_inner_radius: f32,
    pub dog_radius_ratio: f32,
    pub dog_threshold: f32,
    pub dog_sharpness: f32,
    pub offset_variation: f32,
    pub width_variation: f32,
    pub offset_frequency: f32,
    pub width_frequency: f32,
    pub aggressiveness: f32,
    pub noise_seed: f32,
    pub noise_evolution: f32,
}

fn default_toon_outline_dog_inner_radius() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_DOG_INNER_RADIUS)
}

fn default_toon_outline_dog_radius_ratio() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_DOG_RADIUS_RATIO)
}

fn default_toon_outline_dog_threshold() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_DOG_THRESHOLD)
}

fn default_toon_outline_dog_sharpness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_DOG_SHARPNESS)
}

fn default_toon_outline_offset_variation() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_OFFSET_VARIATION)
}

fn default_toon_outline_width_variation() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_WIDTH_VARIATION)
}

fn default_toon_outline_offset_frequency() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_OFFSET_FREQUENCY)
}

fn default_toon_outline_width_frequency() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_WIDTH_FREQUENCY)
}

fn default_toon_outline_aggressiveness() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_AGGRESSIVENESS)
}

fn default_toon_outline_noise_seed() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_NOISE_SEED)
}

fn default_toon_outline_noise_evolution() -> TimelineValue<f32> {
    TimelineValue::<f32>::new_const(DEFAULT_TOON_OUTLINE_NOISE_EVOLUTION)
}

#[derive(Clone, Copy, Debug)]
pub struct ResolvedShadowReceiverPlane3d {
    pub enabled: bool,
    pub composite_enabled: bool,
    pub intensity: f32,
    pub position: Vec3,
    pub rotation_degrees: Vec3,
    pub opacity: f32,
    pub shadow_strength: f32,
    pub reflection: f32,
    pub roughness: f32,
}

#[derive(Clone, Debug)]
pub struct ResolvedEnvironment3d {
    pub source: EnvironmentSource,
    pub file: Option<Asset>,
    pub solid_color: Color<u8>,
    pub rotation_degrees: Vec3,
    pub intensity: f32,
}

#[derive(Clone, Debug)]
pub struct ResolvedObjScene {
    pub model: ResolvedTransform3d,
    pub camera: ResolvedCamera3d,
    pub material: ResolvedPbrMaterial,
    pub shadow_receiver: ResolvedShadowReceiverPlane3d,
    pub environment: ResolvedEnvironment3d,
}

impl ResolvedObjScene {
    pub fn preview_control(
        &self,
        canvas_size: glam::Vec2,
    ) -> Option<shrimply_3d_control_core::Control> {
        shrimply_3d_control_core::Control::new(shrimply_3d_control_core::ControlInput {
            model: self.model,
            camera_position: self.camera.position,
            camera_rotation_degrees: self.camera.rotation_degrees,
            projection: self.camera.projection,
            vertical_fov_degrees: self.camera.vertical_fov_degrees,
            orthographic_height: self.camera.orthographic_height,
            canvas_size,
        })
    }
}
