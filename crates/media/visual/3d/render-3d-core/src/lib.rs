mod math;

use std::{
    fmt,
    mem::size_of,
    path::{Path, PathBuf},
    sync::Arc,
};

use ffmpeg::{format, media};
use ffmpeg_next as ffmpeg;
use glam::{Mat4, Vec3};
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_math_color::Color;

pub use math::{SoftwareBvhNode, resolve_scene_uniforms, software_bvh};
pub use shrimply_transform_3d::Projection as SceneProjection;

include!(concat!(env!("OUT_DIR"), "/slang_bindings.rs"));

const MAX_ENVIRONMENT_WIDTH: u32 = 2048;
pub const MAX_SCENE_LIGHTS: usize = 32;
pub const MAX_SCENE_GROUNDS: usize = 32;

#[derive(Debug)]
pub struct Render3dError(String);

impl Render3dError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Render3dError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Render3dError {}

#[derive(Clone, Debug)]
pub struct PointLightParams {
    pub position: Vec3,
    pub color_linear: Color,
    pub intensity: f32,
    pub range: f32,
    pub radius: f32,
}

#[derive(Clone, Debug)]
pub struct SunLightParams {
    pub rotation_degrees: Vec3,
    pub color_linear: Color,
    pub intensity: f32,
    pub angular_radius_degrees: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundShape {
    Infinite,
    Square,
}

#[derive(Clone, Debug)]
pub struct GroundParams {
    pub shape: GroundShape,
    pub size: f32,
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
pub struct SurfaceMaterialParams {
    pub base_color_linear: Color,
    pub metallic: f32,
    pub roughness: f32,
    normal_mode: obj::NormalMode,
    pbr: obj::PbrSettings,
    identity: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct SceneRenderParams {
    pub model_position: Vec3,
    pub model_anchor: Vec3,
    pub model_rotation_degrees: Vec3,
    pub model_rotation_order: shrimply_scene_3d::RotationOrder,
    pub model_scale: Vec3,
    pub camera_projection: SceneProjection,
    pub anti_aliasing: obj::AntiAliasing,
    pub camera_position: Vec3,
    pub camera_rotation_degrees: Vec3,
    pub vertical_fov_degrees: f32,
    pub orthographic_height: f32,
    pub focus_distance: f32,
    pub background_distance: f32,
    pub background_plane_enabled: bool,
    pub background_intensity: f32,
    pub background_address_mode: shrimply_scene_3d::BackgroundAddressMode,
    pub f_stop: f32,
    pub exposure_ev: f32,
    pub base_color_linear: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub subsurface: f32,
    pub clearcoat: f32,
    pub sheen: f32,
    pub transmission: f32,
    pub ior: f32,
    pub path_tracing: obj::PathTracingMode,
    pub light_sampling_quality: obj::LightSamplingQuality,
    pub denoising: shrimply_scene_3d::DenoisingMethod,
    pub render_quality: obj::RenderQuality,
    pub normal_mode: obj::NormalMode,
    pub shading_model: obj::ShadingModel,
    pub toon_bands: f32,
    pub toon_texture_filter: obj::ToonTextureFilter,
    pub toon_color_levels: f32,
    pub toon_kuwahara_radius: f32,
    pub toon_kuwahara_strength: f32,
    pub toon_shadow_kind: obj::ToonShadowKind,
    pub toon_shadow_color_linear: Color,
    pub toon_shadow_strength: f32,
    pub toon_shadow_darkest_tone: f32,
    pub toon_shadow_dot_size: f32,
    pub toon_shadow_dot_density: f32,
    pub toon_shadow_dot_distribution_randomness: f32,
    pub toon_shadow_dot_size_randomness: f32,
    pub toon_shadow_line_direction_degrees: f32,
    pub toon_shadow_line_width: f32,
    pub toon_shadow_line_density: f32,
    pub toon_shadow_line_distribution_randomness: f32,
    pub toon_shadow_line_width_randomness: f32,
    pub toon_shadow_pattern_softness: f32,
    pub toon_shadow_crosshatch_angle_degrees: f32,
    pub toon_shadow_crosshatch_max_directions: f32,
    pub toon_rim_color_linear: Color,
    pub toon_rim_strength: f32,
    pub toon_rim_power: f32,
    pub toon_specular_size: f32,
    pub toon_specular_strength: f32,
    pub toon_outline_mode: obj::OutlineMode,
    pub toon_outline_method: obj::OutlineMethod,
    pub toon_outline_quality: obj::OutlineQuality,
    pub toon_outline_color_linear: Color,
    pub toon_outline_width: f32,
    pub toon_outline_opacity: f32,
    pub toon_outline_depth_threshold: f32,
    pub toon_outline_normal_angle_degrees: f32,
    pub toon_outline_dog_inner_radius: f32,
    pub toon_outline_dog_radius_ratio: f32,
    pub toon_outline_dog_threshold: f32,
    pub toon_outline_dog_sharpness: f32,
    pub toon_outline_offset_variation: f32,
    pub toon_outline_width_variation: f32,
    pub toon_outline_offset_frequency: f32,
    pub toon_outline_width_frequency: f32,
    pub toon_outline_aggressiveness: f32,
    pub toon_outline_noise_seed: u32,
    pub toon_outline_noise_evolution: f32,
    pub point_lights: Vec<PointLightParams>,
    pub sun_lights: Vec<SunLightParams>,
    pub grounds: Vec<GroundParams>,
    pub ground_shading_model: obj::ShadingModel,
    pub ground_toon_shadow_kind: obj::ToonShadowKind,
    pub ground_toon_shadow_color_linear: Color,
    pub ground_toon_shadow_strength: f32,
    pub shadow_receiver_enabled: bool,
    pub ground_composite_enabled: bool,
    pub ground_intensity: f32,
    pub shadow_receiver_position: Vec3,
    pub shadow_receiver_rotation_degrees: Vec3,
    pub shadow_receiver_opacity: f32,
    pub ground_shadow_strength: f32,
    pub ground_reflection: f32,
    pub ground_roughness: f32,
    pub environment_source: shrimply_scene_3d::EnvironmentSource,
    pub environment_file: Option<Asset>,
    pub environment_solid_color_linear: Color,
    pub environment_rotation_degrees: Vec3,
    pub environment_intensity: f32,
}

impl SceneRenderParams {
    pub fn uniforms(&self, width: u32, height: u32) -> Result<obj::SceneUniforms, Render3dError> {
        resolve_scene_uniforms(width, height, self)
    }
}

impl From<&shrimply_scene_3d::ResolvedObjScene> for SceneRenderParams {
    fn from(scene: &shrimply_scene_3d::ResolvedObjScene) -> Self {
        use shrimply_scene_3d::{
            AntiAliasing, LightSamplingQuality, NormalMode, ShadingModel, ToonOutlineMethod,
            ToonOutlineMode, ToonOutlineQuality, ToonShadowKind, ToonTextureFilter,
        };
        Self {
            model_position: scene.model.position,
            model_anchor: scene.model.anchor,
            model_rotation_degrees: scene.model.rotation_degrees,
            model_rotation_order: scene.model.rotation_order,
            model_scale: scene.model.scale,
            camera_projection: scene.camera.projection,
            anti_aliasing: match scene.camera.anti_aliasing {
                AntiAliasing::None => obj::AntiAliasing::None,
                AntiAliasing::RotatedGrid2x => obj::AntiAliasing::RotatedGrid2x,
                AntiAliasing::Grid4x => obj::AntiAliasing::Grid4x,
                AntiAliasing::Stochastic8x => obj::AntiAliasing::Stochastic8x,
            },
            camera_position: scene.camera.position,
            camera_rotation_degrees: scene.camera.rotation_degrees,
            vertical_fov_degrees: scene.camera.vertical_fov_degrees,
            orthographic_height: scene.camera.orthographic_height,
            focus_distance: scene.camera.focus_distance,
            background_distance: scene.camera.background_distance,
            background_plane_enabled: scene.camera.background_plane_enabled,
            background_intensity: scene.camera.background_intensity,
            background_address_mode: scene.camera.background_address_mode,
            f_stop: scene.camera.f_stop,
            exposure_ev: scene.camera.exposure_ev,
            base_color_linear: scene.material.base_color.to_linear(),
            metallic: scene.material.metallic,
            roughness: scene.material.roughness,
            subsurface: scene.material.subsurface,
            clearcoat: scene.material.clearcoat,
            sheen: scene.material.sheen,
            transmission: scene.material.transmission,
            ior: scene.material.ior,
            path_tracing: match scene.material.path_tracing {
                shrimply_scene_3d::PathTracingMode::Off => obj::PathTracingMode::Off,
                shrimply_scene_3d::PathTracingMode::Samples1 => obj::PathTracingMode::Samples1,
                shrimply_scene_3d::PathTracingMode::Samples2 => obj::PathTracingMode::Samples2,
                shrimply_scene_3d::PathTracingMode::Preview => obj::PathTracingMode::Preview,
                shrimply_scene_3d::PathTracingMode::Samples8 => obj::PathTracingMode::Samples8,
                shrimply_scene_3d::PathTracingMode::Quality => obj::PathTracingMode::Quality,
                shrimply_scene_3d::PathTracingMode::Samples32 => obj::PathTracingMode::Samples32,
                shrimply_scene_3d::PathTracingMode::Samples64 => obj::PathTracingMode::Samples64,
                shrimply_scene_3d::PathTracingMode::Samples128 => obj::PathTracingMode::Samples128,
            },
            light_sampling_quality: match scene.material.light_sampling_quality {
                LightSamplingQuality::Rays1 => obj::LightSamplingQuality::Rays1,
                LightSamplingQuality::Rays2 => obj::LightSamplingQuality::Rays2,
                LightSamplingQuality::Standard => obj::LightSamplingQuality::Standard,
                LightSamplingQuality::High => obj::LightSamplingQuality::High,
                LightSamplingQuality::Ultra => obj::LightSamplingQuality::Ultra,
                LightSamplingQuality::Rays32 => obj::LightSamplingQuality::Rays32,
                LightSamplingQuality::Rays64 => obj::LightSamplingQuality::Rays64,
            },
            denoising: scene.material.denoising,
            render_quality: obj::RenderQuality::Final,
            normal_mode: match scene.material.normal_mode {
                NormalMode::Smooth => obj::NormalMode::Smooth,
                NormalMode::Spherical => obj::NormalMode::Spherical,
                NormalMode::PnTriangle => obj::NormalMode::PnTriangle,
                NormalMode::Flat => obj::NormalMode::Flat,
            },
            shading_model: match scene.material.shading_model {
                ShadingModel::Pbr => obj::ShadingModel::Pbr,
                ShadingModel::Toon => obj::ShadingModel::Toon,
                ShadingModel::Depth => obj::ShadingModel::Depth,
            },
            toon_bands: scene.material.toon.bands,
            toon_texture_filter: match scene.material.toon.texture_filter {
                ToonTextureFilter::Direct => obj::ToonTextureFilter::Direct,
                ToonTextureFilter::Kuwahara => obj::ToonTextureFilter::Kuwahara,
            },
            toon_color_levels: scene.material.toon.color_levels,
            toon_kuwahara_radius: scene.material.toon.kuwahara_radius,
            toon_kuwahara_strength: scene.material.toon.kuwahara_strength,
            toon_shadow_kind: match scene.material.toon.shadow_kind {
                ToonShadowKind::Solid => obj::ToonShadowKind::Solid,
                ToonShadowKind::Dots => obj::ToonShadowKind::Dots,
                ToonShadowKind::Lines => obj::ToonShadowKind::Lines,
                ToonShadowKind::Crosshatch => obj::ToonShadowKind::Crosshatch,
            },
            toon_shadow_color_linear: scene.material.toon.shadow_color.to_linear(),
            toon_shadow_strength: scene.material.toon.shadow_strength,
            toon_shadow_darkest_tone: scene.material.toon.shadow_darkest_tone,
            toon_shadow_dot_size: scene.material.toon.shadow_dot_size,
            toon_shadow_dot_density: scene.material.toon.shadow_dot_density,
            toon_shadow_dot_distribution_randomness: scene
                .material
                .toon
                .shadow_dot_distribution_randomness,
            toon_shadow_dot_size_randomness: scene.material.toon.shadow_dot_size_randomness,
            toon_shadow_line_direction_degrees: scene.material.toon.shadow_line_direction_degrees,
            toon_shadow_line_width: scene.material.toon.shadow_line_width,
            toon_shadow_line_density: scene.material.toon.shadow_line_density,
            toon_shadow_line_distribution_randomness: scene
                .material
                .toon
                .shadow_line_distribution_randomness,
            toon_shadow_line_width_randomness: scene.material.toon.shadow_line_width_randomness,
            toon_shadow_pattern_softness: scene.material.toon.shadow_pattern_softness,
            toon_shadow_crosshatch_angle_degrees: scene
                .material
                .toon
                .shadow_crosshatch_angle_degrees,
            toon_shadow_crosshatch_max_directions: scene
                .material
                .toon
                .shadow_crosshatch_max_directions,
            toon_rim_color_linear: scene.material.toon.rim_color.to_linear(),
            toon_rim_strength: scene.material.toon.rim_strength,
            toon_rim_power: scene.material.toon.rim_power,
            toon_specular_size: scene.material.toon.specular_size,
            toon_specular_strength: scene.material.toon.specular_strength,
            toon_outline_mode: match scene.material.toon.outline.mode {
                ToonOutlineMode::Off => obj::OutlineMode::Off,
                ToonOutlineMode::Silhouette => obj::OutlineMode::Silhouette,
                ToonOutlineMode::SilhouetteAndCreases => obj::OutlineMode::SilhouetteAndCreases,
            },
            toon_outline_method: match scene.material.toon.outline.method {
                ToonOutlineMethod::RayTraced => obj::OutlineMethod::RayTraced,
                ToonOutlineMethod::Fresnel => obj::OutlineMethod::Fresnel,
                ToonOutlineMethod::Hybrid => obj::OutlineMethod::Hybrid,
                ToonOutlineMethod::Sobel => obj::OutlineMethod::Sobel,
                ToonOutlineMethod::RobertsCross => obj::OutlineMethod::RobertsCross,
                ToonOutlineMethod::DifferenceOfGaussians => {
                    obj::OutlineMethod::DifferenceOfGaussians
                }
                ToonOutlineMethod::RegionBoundary => obj::OutlineMethod::RegionBoundary,
            },
            toon_outline_quality: match scene.material.toon.outline.quality {
                ToonOutlineQuality::Standard => obj::OutlineQuality::Standard,
                ToonOutlineQuality::High => obj::OutlineQuality::High,
                ToonOutlineQuality::Ultra => obj::OutlineQuality::Ultra,
            },
            toon_outline_color_linear: scene.material.toon.outline.color.to_linear(),
            toon_outline_width: scene.material.toon.outline.width,
            toon_outline_opacity: scene.material.toon.outline.opacity,
            toon_outline_depth_threshold: scene.material.toon.outline.depth_threshold,
            toon_outline_normal_angle_degrees: scene.material.toon.outline.normal_angle_degrees,
            toon_outline_dog_inner_radius: scene.material.toon.outline.dog_inner_radius,
            toon_outline_dog_radius_ratio: scene.material.toon.outline.dog_radius_ratio,
            toon_outline_dog_threshold: scene.material.toon.outline.dog_threshold,
            toon_outline_dog_sharpness: scene.material.toon.outline.dog_sharpness,
            toon_outline_offset_variation: scene.material.toon.outline.offset_variation,
            toon_outline_width_variation: scene.material.toon.outline.width_variation,
            toon_outline_offset_frequency: scene.material.toon.outline.offset_frequency,
            toon_outline_width_frequency: scene.material.toon.outline.width_frequency,
            toon_outline_aggressiveness: scene.material.toon.outline.aggressiveness,
            toon_outline_noise_seed: scene
                .material
                .toon
                .outline
                .noise_seed
                .round()
                .clamp(0.0, u32::MAX as f32) as u32,
            toon_outline_noise_evolution: scene.material.toon.outline.noise_evolution,
            point_lights: Vec::new(),
            sun_lights: Vec::new(),
            grounds: scene
                .shadow_receiver
                .enabled
                .then_some(GroundParams {
                    shape: GroundShape::Infinite,
                    size: 0.0,
                    composite_enabled: scene.shadow_receiver.composite_enabled,
                    intensity: scene.shadow_receiver.intensity,
                    position: scene.shadow_receiver.position,
                    rotation_degrees: scene.shadow_receiver.rotation_degrees,
                    opacity: scene.shadow_receiver.opacity,
                    shadow_strength: scene.shadow_receiver.shadow_strength,
                    reflection: scene.shadow_receiver.reflection,
                    roughness: scene.shadow_receiver.roughness,
                })
                .into_iter()
                .collect(),
            ground_shading_model: match scene.material.shading_model {
                ShadingModel::Pbr => obj::ShadingModel::Pbr,
                ShadingModel::Toon => obj::ShadingModel::Toon,
                ShadingModel::Depth => obj::ShadingModel::Depth,
            },
            ground_toon_shadow_kind: match scene.material.toon.shadow_kind {
                ToonShadowKind::Solid => obj::ToonShadowKind::Solid,
                ToonShadowKind::Dots => obj::ToonShadowKind::Dots,
                ToonShadowKind::Lines => obj::ToonShadowKind::Lines,
                ToonShadowKind::Crosshatch => obj::ToonShadowKind::Crosshatch,
            },
            ground_toon_shadow_color_linear: scene.material.toon.shadow_color.to_linear(),
            ground_toon_shadow_strength: scene.material.toon.shadow_strength,
            shadow_receiver_enabled: scene.shadow_receiver.enabled,
            ground_composite_enabled: scene.shadow_receiver.composite_enabled,
            ground_intensity: scene.shadow_receiver.intensity,
            shadow_receiver_position: scene.shadow_receiver.position,
            shadow_receiver_rotation_degrees: scene.shadow_receiver.rotation_degrees,
            shadow_receiver_opacity: scene.shadow_receiver.opacity,
            ground_shadow_strength: scene.shadow_receiver.shadow_strength,
            ground_reflection: scene.shadow_receiver.reflection,
            ground_roughness: scene.shadow_receiver.roughness,
            environment_source: scene.environment.source,
            environment_file: scene.environment.file.clone(),
            environment_solid_color_linear: scene.environment.solid_color.to_linear(),
            environment_rotation_degrees: scene.environment.rotation_degrees,
            environment_intensity: scene.environment.intensity,
        }
    }
}

impl From<&shrimply_scene_3d::ResolvedObjScene> for SurfaceMaterialParams {
    fn from(scene: &shrimply_scene_3d::ResolvedObjScene) -> Self {
        Self::from(&SceneRenderParams::from(scene))
    }
}

impl From<&SceneRenderParams> for SurfaceMaterialParams {
    fn from(params: &SceneRenderParams) -> Self {
        let mut pbr = math::pbr_settings(params);
        pbr.path_tracing = obj::PathTracingMode::Off;
        pbr.denoising = 0;
        pbr.transmission = params.transmission.clamp(0.0, 1.0);
        let mut identity = Vec::new();
        identity.extend(params.base_color_linear.to_array().map(f32::to_bits));
        identity.extend(
            [
                params.metallic,
                params.roughness,
                pbr.subsurface,
                pbr.clearcoat,
                pbr.sheen,
                pbr.transmission,
                pbr.ior,
            ]
            .map(f32::to_bits),
        );
        identity.push(params.normal_mode as u32);
        Self {
            base_color_linear: params.base_color_linear,
            metallic: params.metallic,
            roughness: params.roughness,
            normal_mode: params.normal_mode,
            pbr,
            identity,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SceneIdentity {
    sources: Vec<AssetSnapshot>,
    geometry: Vec<u32>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct GeometryIdentity(Vec<(SceneIdentity, bool)>);

#[derive(Clone, Copy, Debug)]
pub struct MeshGeometry {
    pub vertex_offsets: [u32; 2],
    pub primitive_counts: [u32; 2],
    pub opaque: [bool; 2],
    pub geometry_count: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct AccelerationInstance {
    pub geometry_index: u32,
    pub transform: [f32; 16],
}

pub struct ObjRenderSession {
    path: PathBuf,
    source_identity: Option<AssetSnapshot>,
    identity: SceneIdentity,
    geometry_identity: GeometryIdentity,
    mesh: shrimply_scene_3d::ObjMesh,
    materials: Vec<obj::MeshMaterial>,
    geometries: Vec<MeshGeometry>,
    acceleration_instances: Vec<AccelerationInstance>,
    mesh_instances: Vec<obj::MeshInstance>,
}

pub struct SceneObject<'a> {
    pub session: &'a ObjRenderSession,
    pub transform: shrimply_scene_3d::ResolvedTransform3d,
    pub material: SurfaceMaterialParams,
}

pub struct PreparedFrame {
    pub session: Arc<ObjRenderSession>,
    pub params: SceneRenderParams,
    pub uniforms: obj::SceneUniforms,
    pub environment: Option<AssetSnapshot>,
    pub width: u32,
    pub height: u32,
}

impl ObjRenderSession {
    pub fn load(asset: &Asset) -> Result<Self, Render3dError> {
        let identity = asset.snapshot().map_err(Render3dError::message)?;
        let mesh = if identity
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("glb"))
        {
            shrimply_scene_3d::load_glb(identity.path()).map_err(|error| {
                Render3dError::message(format!("parse {}: {error}", identity.path().display()))
            })?
        } else {
            shrimply_scene_3d::load_obj(identity.path()).map_err(|error| {
                Render3dError::message(format!("parse {}: {error}", identity.path().display()))
            })?
        };
        identity.verify_current().map_err(Render3dError::message)?;
        let materials = mesh
            .materials
            .iter()
            .map(|material| obj::MeshMaterial {
                base_color_factor: material.base_color_factor.to_array(),
                metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                flags: material.flags,
                base_color_texture: reflected_texture_mapping(material.base_color_texture),
                metallic_roughness_texture: reflected_texture_mapping(
                    material.metallic_roughness_texture,
                ),
                normal_texture: reflected_texture_mapping(material.normal_texture),
                surface: [0; 4],
                pbr: Default::default(),
                toon: Default::default(),
            })
            .collect();
        let scene_identity = SceneIdentity {
            sources: vec![identity.clone()],
            geometry: Vec::new(),
        };
        let vertex_count = mesh.positions.len() as u32;
        Ok(Self {
            path: identity.path().to_path_buf(),
            source_identity: Some(identity.clone()),
            geometry_identity: GeometryIdentity(vec![(scene_identity.clone(), true)]),
            identity: scene_identity,
            mesh,
            materials,
            geometries: vec![MeshGeometry {
                vertex_offsets: [0, 0],
                primitive_counts: [vertex_count / 3, 0],
                opaque: [false, false],
                geometry_count: 1,
            }],
            acceleration_instances: vec![AccelerationInstance {
                geometry_index: 0,
                transform: Mat4::IDENTITY.to_cols_array(),
            }],
            mesh_instances: vec![obj::MeshInstance {
                model: Mat4::IDENTITY.to_cols_array(),
                normal: Mat4::IDENTITY.to_cols_array(),
                geometry_offsets: [0; 4],
            }],
        })
    }

    pub fn generated(
        label: impl Into<PathBuf>,
        geometry: Vec<u32>,
        mesh: shrimply_scene_3d::ObjMesh,
    ) -> Self {
        let materials = mesh
            .materials
            .iter()
            .map(|material| obj::MeshMaterial {
                base_color_factor: material.base_color_factor.to_array(),
                metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                flags: material.flags,
                base_color_texture: reflected_texture_mapping(material.base_color_texture),
                metallic_roughness_texture: reflected_texture_mapping(
                    material.metallic_roughness_texture,
                ),
                normal_texture: reflected_texture_mapping(material.normal_texture),
                surface: [0; 4],
                pbr: Default::default(),
                toon: Default::default(),
            })
            .collect();
        let scene_identity = SceneIdentity {
            sources: Vec::new(),
            geometry,
        };
        let vertex_count = mesh.positions.len() as u32;
        Self {
            path: label.into(),
            source_identity: None,
            geometry_identity: GeometryIdentity(vec![(scene_identity.clone(), true)]),
            identity: scene_identity,
            mesh,
            materials,
            geometries: vec![MeshGeometry {
                vertex_offsets: [0, 0],
                primitive_counts: [vertex_count / 3, 0],
                opaque: [false, false],
                geometry_count: 1,
            }],
            acceleration_instances: vec![AccelerationInstance {
                geometry_index: 0,
                transform: Mat4::IDENTITY.to_cols_array(),
            }],
            mesh_instances: vec![obj::MeshInstance {
                model: Mat4::IDENTITY.to_cols_array(),
                normal: Mat4::IDENTITY.to_cols_array(),
                geometry_offsets: [0; 4],
            }],
        }
    }

    pub fn compose(objects: &[SceneObject<'_>]) -> Result<Self, Render3dError> {
        let identity = SceneIdentity::for_objects(objects);
        let mut unique_geometry = Vec::new();
        let mut object_geometry_indices = Vec::with_capacity(objects.len());
        for object in objects {
            let force_any_hit = object.material.pbr.transmission > 0.0;
            let key = (object.session.identity.clone(), force_any_hit);
            let index = unique_geometry
                .iter()
                .position(|(_, _, existing)| existing == &key)
                .unwrap_or_else(|| {
                    unique_geometry.push((object.session, force_any_hit, key));
                    unique_geometry.len() - 1
                });
            object_geometry_indices.push(index);
        }
        let mut geometry_identity = GeometryIdentity(
            unique_geometry
                .iter()
                .map(|(_, _, key)| key.clone())
                .collect(),
        );
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut tangents = Vec::new();
        let mut tex_coords_0 = Vec::new();
        let mut tex_coords_1 = Vec::new();
        let mut colors = Vec::new();
        let mut geometries = Vec::with_capacity(unique_geometry.len());
        let atlas_width = unique_geometry
            .iter()
            .try_fold(0_u32, |width, (session, _, _)| {
                width.checked_add(session.mesh.texture_atlas.width)
            })
            .ok_or_else(|| Render3dError::message("combined material atlas width overflow"))?
            .max(1);
        let atlas_height = unique_geometry
            .iter()
            .map(|(session, _, _)| session.mesh.texture_atlas.height)
            .max()
            .unwrap_or(1)
            .max(1);
        let atlas_len = usize::try_from(u64::from(atlas_width) * u64::from(atlas_height))
            .map_err(|_| Render3dError::message("combined material atlas is too large"))?;
        let mut atlas_pixels = vec![Color::<u8>::WHITE; atlas_len];
        let mut atlas_x = 0_u32;
        let mut geometry_atlas_x = Vec::with_capacity(unique_geometry.len());

        for (session, force_any_hit, _) in &unique_geometry {
            let mesh = &session.mesh;
            if mesh.positions.len() != mesh.materials.len() * 3
                || mesh.normals.len() != mesh.positions.len()
                || mesh.tangents.len() != mesh.positions.len()
                || mesh.tex_coords_0.len() != mesh.positions.len()
                || mesh.tex_coords_1.len() != mesh.positions.len()
                || mesh.colors.len() != mesh.positions.len()
            {
                return Err(Render3dError::message(
                    "3D mesh attribute and material counts do not match",
                ));
            }
            let mut geometry = MeshGeometry {
                vertex_offsets: [0; 2],
                primitive_counts: [0; 2],
                opaque: [false; 2],
                geometry_count: 0,
            };
            for opaque in [true, false] {
                let vertex_offset = u32::try_from(positions.len())
                    .map_err(|_| Render3dError::message("3D vertex offset overflow"))?;
                let mut primitive_count = 0_u32;
                for (triangle, material) in mesh.materials.iter().enumerate() {
                    let triangle_opaque = !*force_any_hit && material.flags[1] == 0;
                    if triangle_opaque != opaque {
                        continue;
                    }
                    let first = triangle * 3;
                    let vertices = first..first + 3;
                    positions.extend_from_slice(&mesh.positions[vertices.clone()]);
                    normals.extend_from_slice(&mesh.normals[vertices.clone()]);
                    tangents.extend_from_slice(&mesh.tangents[vertices.clone()]);
                    tex_coords_0.extend_from_slice(&mesh.tex_coords_0[vertices.clone()]);
                    tex_coords_1.extend_from_slice(&mesh.tex_coords_1[vertices.clone()]);
                    colors.extend_from_slice(&mesh.colors[vertices]);
                    primitive_count = primitive_count
                        .checked_add(1)
                        .ok_or_else(|| Render3dError::message("3D primitive count overflow"))?;
                }
                if primitive_count == 0 {
                    continue;
                }
                let slot = geometry.geometry_count as usize;
                geometry.vertex_offsets[slot] = vertex_offset;
                geometry.primitive_counts[slot] = primitive_count;
                geometry.opaque[slot] = opaque;
                geometry.geometry_count += 1;
            }
            if geometry.geometry_count == 0 {
                return Err(Render3dError::message("3D object has no triangles"));
            }
            geometries.push(geometry);
            geometry_atlas_x.push(atlas_x);

            let source_atlas = &mesh.texture_atlas;
            for y in 0..source_atlas.height {
                let source_start = usize::try_from(u64::from(y) * u64::from(source_atlas.width))
                    .map_err(|_| Render3dError::message("material atlas row overflow"))?;
                let target_start =
                    usize::try_from(u64::from(y) * u64::from(atlas_width) + u64::from(atlas_x))
                        .map_err(|_| {
                            Render3dError::message("combined material atlas row overflow")
                        })?;
                let count = source_atlas.width as usize;
                atlas_pixels[target_start..target_start + count]
                    .copy_from_slice(&source_atlas.pixels[source_start..source_start + count]);
            }
            atlas_x += source_atlas.width;
        }

        let mut bounds_positions = Vec::new();
        for object in objects {
            let transform = object_matrix(object.transform);
            if !transform.is_finite() || !transform.inverse().is_finite() {
                return Err(Render3dError::message("3D object transform is not finite"));
            }
            bounds_positions.extend(object.session.mesh.positions.iter().map(|position| {
                transform
                    .transform_point3(Vec3::from_array(position[..3].try_into().unwrap()))
                    .extend(1.0)
                    .to_array()
            }));
        }
        if positions.is_empty() {
            positions.extend([
                [-1.0, -1.0, -1_000_000.0, 1.0],
                [1.0, -1.0, -1_000_000.0, 1.0],
                [0.0, 1.0, -1_000_000.0, 1.0],
            ]);
            normals.extend([[0.0, 0.0, 1.0, 0.0]; 3]);
            tangents.extend([[1.0, 0.0, 0.0, 1.0]; 3]);
            tex_coords_0.extend([[0.0; 4]; 3]);
            tex_coords_1.extend([[0.0; 4]; 3]);
            colors.extend([Color::WHITE; 3]);
            bounds_positions.extend_from_slice(&positions);
            geometries.push(MeshGeometry {
                vertex_offsets: [0, 0],
                primitive_counts: [1, 0],
                opaque: [false, false],
                geometry_count: 1,
            });
            geometry_identity = GeometryIdentity(vec![(identity.clone(), true)]);
        }

        let (center, radius) = combined_bounds(&bounds_positions)?;
        let normalization =
            Mat4::from_scale(Vec3::splat(radius.recip())) * Mat4::from_translation(-center);
        let mut reflected_materials = Vec::new();
        let mut acceleration_instances = Vec::with_capacity(objects.len().max(1));
        let mut mesh_instances = Vec::with_capacity(objects.len().max(1));
        for (object, geometry_index) in objects.iter().zip(object_geometry_indices) {
            let geometry = geometries[geometry_index];
            let source_atlas = &object.session.mesh.texture_atlas;
            let force_any_hit = object.material.pbr.transmission > 0.0;
            let material_offset = u32::try_from(reflected_materials.len())
                .map_err(|_| Render3dError::message("3D material offset overflow"))?;
            for slot in 0..geometry.geometry_count as usize {
                for source in &object.session.mesh.materials {
                    let triangle_opaque = !force_any_hit && source.flags[1] == 0;
                    if triangle_opaque != geometry.opaque[slot] {
                        continue;
                    }
                    let mut material = *source;
                    let atlas_x = geometry_atlas_x[geometry_index];
                    remap_texture(
                        &mut material.base_color_texture,
                        atlas_x,
                        source_atlas.width,
                        source_atlas.height,
                        atlas_width,
                        atlas_height,
                    );
                    remap_texture(
                        &mut material.metallic_roughness_texture,
                        atlas_x,
                        source_atlas.width,
                        source_atlas.height,
                        atlas_width,
                        atlas_height,
                    );
                    remap_texture(
                        &mut material.normal_texture,
                        atlas_x,
                        source_atlas.width,
                        source_atlas.height,
                        atlas_width,
                        atlas_height,
                    );
                    material.base_color_factor *= object.material.base_color_linear;
                    material.metallic_roughness_normal_alpha[0] *= object.material.metallic;
                    material.metallic_roughness_normal_alpha[1] *= object.material.roughness;
                    material.flags[0] = 1;
                    material.flags[3] = 1;
                    reflected_materials.push(obj::MeshMaterial {
                        base_color_factor: material.base_color_factor.to_array(),
                        metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                        flags: material.flags,
                        base_color_texture: reflected_texture_mapping(material.base_color_texture),
                        metallic_roughness_texture: reflected_texture_mapping(
                            material.metallic_roughness_texture,
                        ),
                        normal_texture: reflected_texture_mapping(material.normal_texture),
                        surface: [object.material.normal_mode as u32, 0, 0, 0],
                        pbr: object.material.pbr,
                        toon: Default::default(),
                    });
                }
            }
            let local_to_scene = normalization * object_matrix(object.transform);
            let normal = local_to_scene.inverse().transpose();
            if !local_to_scene.is_finite() || !normal.is_finite() {
                return Err(Render3dError::message("3D object transform is not finite"));
            }
            acceleration_instances.push(AccelerationInstance {
                geometry_index: geometry_index as u32,
                transform: local_to_scene.to_cols_array(),
            });
            let second_material = material_offset + geometry.primitive_counts[0];
            mesh_instances.push(obj::MeshInstance {
                model: local_to_scene.to_cols_array(),
                normal: normal.to_cols_array(),
                geometry_offsets: [
                    geometry.vertex_offsets[0],
                    material_offset,
                    geometry.vertex_offsets[1],
                    second_material,
                ],
            });
        }
        if objects.is_empty() {
            let mut material = shrimply_scene_3d::MeshMaterial::default();
            material.base_color_factor.a = 0.0;
            material.flags = [1, 2, 0, 0];
            reflected_materials.push(obj::MeshMaterial {
                base_color_factor: material.base_color_factor.to_array(),
                metallic_roughness_normal_alpha: material.metallic_roughness_normal_alpha,
                flags: material.flags,
                base_color_texture: reflected_texture_mapping(material.base_color_texture),
                metallic_roughness_texture: reflected_texture_mapping(
                    material.metallic_roughness_texture,
                ),
                normal_texture: reflected_texture_mapping(material.normal_texture),
                surface: [0; 4],
                pbr: Default::default(),
                toon: Default::default(),
            });
            acceleration_instances.push(AccelerationInstance {
                geometry_index: 0,
                transform: normalization.to_cols_array(),
            });
            mesh_instances.push(obj::MeshInstance {
                model: normalization.to_cols_array(),
                normal: normalization.inverse().transpose().to_cols_array(),
                geometry_offsets: [0; 4],
            });
        }
        let mesh = shrimply_scene_3d::ObjMesh {
            positions,
            normals,
            tangents,
            tex_coords_0,
            tex_coords_1,
            colors,
            materials: Vec::new(),
            texture_atlas: shrimply_scene_3d::TextureAtlas {
                pixels: atlas_pixels,
                width: atlas_width,
                height: atlas_height,
            },
            face_normals: Vec::new(),
            triangles: Vec::new(),
            source_center: center,
            source_radius: radius,
        };
        Ok(Self {
            path: identity.sources.first().map_or_else(
                || PathBuf::from("<empty 3D scene>"),
                |value| value.path().to_path_buf(),
            ),
            source_identity: None,
            identity,
            geometry_identity,
            mesh,
            materials: reflected_materials,
            geometries,
            acceleration_instances,
            mesh_instances,
        })
    }

    pub fn matches_asset(&self, asset: &Asset) -> Result<bool, Render3dError> {
        Ok(self.source_identity.as_ref()
            == Some(&asset.snapshot().map_err(Render3dError::message)?))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> &SceneIdentity {
        &self.identity
    }

    pub fn geometry_identity(&self) -> &GeometryIdentity {
        &self.geometry_identity
    }

    pub fn geometries(&self) -> &[MeshGeometry] {
        &self.geometries
    }

    pub fn acceleration_instances(&self) -> &[AccelerationInstance] {
        &self.acceleration_instances
    }

    pub fn mesh_instances(&self) -> &[obj::MeshInstance] {
        &self.mesh_instances
    }

    pub fn mesh(&self) -> &shrimply_scene_3d::ObjMesh {
        &self.mesh
    }

    pub fn positions(&self) -> &[[f32; 4]] {
        &self.mesh.positions
    }

    pub fn normals(&self) -> &[[f32; 4]] {
        &self.mesh.normals
    }

    pub fn tangents(&self) -> &[[f32; 4]] {
        &self.mesh.tangents
    }

    pub fn tex_coords_0(&self) -> &[[f32; 4]] {
        &self.mesh.tex_coords_0
    }

    pub fn tex_coords_1(&self) -> &[[f32; 4]] {
        &self.mesh.tex_coords_1
    }

    pub fn colors(&self) -> &[Color] {
        &self.mesh.colors
    }

    pub fn materials(&self) -> &[obj::MeshMaterial] {
        &self.materials
    }

    pub fn texture_atlas(&self) -> &shrimply_scene_3d::TextureAtlas {
        &self.mesh.texture_atlas
    }

    pub fn vertex_count(&self) -> u32 {
        self.mesh.positions.len() as u32
    }
}

impl SceneIdentity {
    pub fn for_objects(objects: &[SceneObject<'_>]) -> Self {
        let sources = objects
            .iter()
            .flat_map(|object| object.session.identity.sources.iter().cloned())
            .collect();
        let mut geometry = Vec::new();
        for object in objects {
            geometry.extend_from_slice(&object.session.identity.geometry);
            geometry.extend(
                object_matrix(object.transform)
                    .to_cols_array()
                    .map(f32::to_bits),
            );
            geometry.extend_from_slice(&object.material.identity);
        }
        Self { sources, geometry }
    }
}

fn object_matrix(transform: shrimply_scene_3d::ResolvedTransform3d) -> Mat4 {
    Mat4::from_scale_rotation_translation(
        transform.scale,
        shrimply_transform_3d::rotation(transform.rotation_degrees, transform.rotation_order),
        transform.position,
    ) * Mat4::from_translation(-transform.anchor)
}

fn combined_bounds(positions: &[[f32; 4]]) -> Result<(Vec3, f32), Render3dError> {
    let first = Vec3::from_array(positions[0][..3].try_into().unwrap());
    let (minimum, maximum) =
        positions
            .iter()
            .skip(1)
            .fold((first, first), |(minimum, maximum), value| {
                let value = Vec3::from_array(value[..3].try_into().unwrap());
                (minimum.min(value), maximum.max(value))
            });
    let center = (minimum + maximum) * 0.5;
    let radius = positions
        .iter()
        .map(|value| Vec3::from_array(value[..3].try_into().unwrap()).distance(center))
        .fold(0.0_f32, f32::max)
        .max(f32::EPSILON);
    if !center.is_finite() || !radius.is_finite() {
        return Err(Render3dError::message(
            "combined 3D scene bounds are not finite",
        ));
    }
    Ok((center, radius))
}

fn remap_texture(
    mapping: &mut shrimply_scene_3d::TextureMapping,
    x: u32,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) {
    mapping.atlas = [
        (x as f32 + mapping.atlas[0] * source_width as f32) / width as f32,
        mapping.atlas[1] * source_height as f32 / height as f32,
        mapping.atlas[2] * source_width as f32 / width as f32,
        mapping.atlas[3] * source_height as f32 / height as f32,
    ];
}

fn reflected_texture_mapping(mapping: shrimply_scene_3d::TextureMapping) -> obj::TextureMapping {
    obj::TextureMapping {
        atlas: mapping.atlas,
        transform: mapping.transform,
        settings: mapping.settings,
        rotation_filter: mapping.rotation_filter,
    }
}

pub struct DecodedEnvironment {
    pub pixels: Vec<Color>,
    pub width: u32,
    pub height: u32,
}

pub fn load_environment(path: impl AsRef<Path>) -> Result<DecodedEnvironment, Render3dError> {
    let path = path.as_ref();
    ffmpeg::init()
        .map_err(|error| Render3dError::message(format!("initialize FFmpeg: {error}")))?;
    let mut input = format::input(path).map_err(|error| {
        Render3dError::message(format!("decode environment {}: {error}", path.display()))
    })?;
    let stream = input.streams().best(media::Type::Video).ok_or_else(|| {
        Render3dError::message(format!(
            "environment {} has no image stream",
            path.display()
        ))
    })?;
    let stream_index = stream.index();
    let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|error| Render3dError::message(error.to_string()))?;
    let mut decoder = context.decoder().video().map_err(|error| {
        Render3dError::message(format!(
            "unsupported environment decoder for {}: {error}",
            path.display()
        ))
    })?;
    let width = decoder.width().clamp(1, MAX_ENVIRONMENT_WIDTH);
    let height = (u64::from(decoder.height()) * u64::from(width)
        / u64::from(decoder.width().max(1)))
    .max(1)
    .try_into()
    .map_err(|_| Render3dError::message("environment height overflow"))?;
    let mut scaler = ffmpeg::software::scaling::context::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        format::Pixel::GBRPF32LE,
        width,
        height,
        ffmpeg::software::scaling::flag::Flags::BILINEAR,
    )
    .map_err(|error| Render3dError::message(format!("create environment scaler: {error}")))?;
    for (packet_stream, packet) in input.packets() {
        if packet_stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|error| Render3dError::message(error.to_string()))?;
        let mut decoded = ffmpeg::frame::Video::empty();
        if decoder.receive_frame(&mut decoded).is_ok() {
            let mut planar = ffmpeg::frame::Video::empty();
            scaler
                .run(&decoded, &mut planar)
                .map_err(|error| Render3dError::message(format!("convert environment: {error}")))?;
            return interleave_environment(path, &planar, width, height);
        }
    }
    decoder
        .send_eof()
        .map_err(|error| Render3dError::message(error.to_string()))?;
    let mut decoded = ffmpeg::frame::Video::empty();
    decoder.receive_frame(&mut decoded).map_err(|error| {
        Render3dError::message(format!(
            "environment {} produced no frame: {error}",
            path.display()
        ))
    })?;
    let mut planar = ffmpeg::frame::Video::empty();
    scaler
        .run(&decoded, &mut planar)
        .map_err(|error| Render3dError::message(format!("convert environment: {error}")))?;
    interleave_environment(path, &planar, width, height)
}

pub fn decode_environment(path: impl AsRef<Path>) -> Result<DecodedEnvironment, Render3dError> {
    load_environment(path)
}

fn interleave_environment(
    path: &Path,
    frame: &ffmpeg::frame::Video,
    width: u32,
    height: u32,
) -> Result<DecodedEnvironment, Render3dError> {
    let linear = matches!(
        path.extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("hdr" | "exr")
    );
    let count = width as usize * height as usize;
    let mut pixels = Vec::with_capacity(count);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let read = |plane: usize| -> Result<f32, Render3dError> {
                let offset = y
                    .checked_mul(frame.stride(plane))
                    .and_then(|value| value.checked_add(x * size_of::<f32>()))
                    .ok_or_else(|| Render3dError::message("environment row overflow"))?;
                let bytes: [u8; 4] = frame
                    .data(plane)
                    .get(offset..offset + size_of::<f32>())
                    .ok_or_else(|| Render3dError::message("short environment frame"))?
                    .try_into()
                    .expect("four bytes");
                Ok(f32::from_le_bytes(bytes))
            };
            let convert = |value: f32| {
                if linear {
                    value
                } else if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            pixels.push(Color::from_rgb(
                convert(read(2)?),
                convert(read(0)?),
                convert(read(1)?),
            ));
        }
    }
    Ok(DecodedEnvironment {
        pixels,
        width,
        height,
    })
}

pub fn validate_scene(
    width: u32,
    height: u32,
    params: &SceneRenderParams,
) -> Result<(), Render3dError> {
    if width == 0 || height == 0 {
        return Err(Render3dError::message(
            "scene canvas dimensions must be nonzero",
        ));
    }
    if params.point_lights.len() > MAX_SCENE_LIGHTS || params.sun_lights.len() > MAX_SCENE_LIGHTS {
        return Err(Render3dError::message(format!(
            "a scene supports at most {MAX_SCENE_LIGHTS} point lights and {MAX_SCENE_LIGHTS} suns"
        )));
    }
    if params.grounds.len() > MAX_SCENE_GROUNDS {
        return Err(Render3dError::message(format!(
            "a scene supports at most {MAX_SCENE_GROUNDS} grounds"
        )));
    }
    let values = [
        params.model_position.x,
        params.model_position.y,
        params.model_position.z,
        params.model_rotation_degrees.x,
        params.model_rotation_degrees.y,
        params.model_rotation_degrees.z,
        params.model_scale.x,
        params.model_scale.y,
        params.model_scale.z,
        params.camera_position.x,
        params.camera_position.y,
        params.camera_position.z,
        params.camera_rotation_degrees.x,
        params.camera_rotation_degrees.y,
        params.camera_rotation_degrees.z,
        params.vertical_fov_degrees,
        params.orthographic_height,
        params.focus_distance,
        params.background_distance,
        params.f_stop,
        params.exposure_ev,
        params.metallic,
        params.roughness,
        params.subsurface,
        params.clearcoat,
        params.sheen,
        params.transmission,
        params.ior,
        params.toon_bands,
        params.toon_color_levels,
        params.toon_kuwahara_radius,
        params.toon_kuwahara_strength,
        params.toon_shadow_strength,
        params.toon_shadow_darkest_tone,
        params.toon_shadow_dot_size,
        params.toon_shadow_dot_density,
        params.toon_shadow_dot_distribution_randomness,
        params.toon_shadow_dot_size_randomness,
        params.toon_shadow_line_direction_degrees,
        params.toon_shadow_line_width,
        params.toon_shadow_line_density,
        params.toon_shadow_line_distribution_randomness,
        params.toon_shadow_line_width_randomness,
        params.toon_shadow_pattern_softness,
        params.toon_shadow_crosshatch_angle_degrees,
        params.toon_shadow_crosshatch_max_directions,
        params.toon_rim_strength,
        params.toon_rim_power,
        params.toon_specular_size,
        params.toon_specular_strength,
        params.toon_outline_width,
        params.toon_outline_opacity,
        params.toon_outline_depth_threshold,
        params.toon_outline_normal_angle_degrees,
        params.toon_outline_dog_inner_radius,
        params.toon_outline_dog_radius_ratio,
        params.toon_outline_dog_threshold,
        params.toon_outline_dog_sharpness,
        params.toon_outline_offset_variation,
        params.toon_outline_width_variation,
        params.toon_outline_offset_frequency,
        params.toon_outline_width_frequency,
        params.toon_outline_aggressiveness,
        params.toon_outline_noise_evolution,
        params.ground_toon_shadow_strength,
        params.shadow_receiver_position.x,
        params.shadow_receiver_position.y,
        params.shadow_receiver_position.z,
        params.shadow_receiver_rotation_degrees.x,
        params.shadow_receiver_rotation_degrees.y,
        params.shadow_receiver_rotation_degrees.z,
        params.shadow_receiver_opacity,
        params.ground_shadow_strength,
        params.ground_reflection,
        params.ground_roughness,
        params.ground_intensity,
        params.background_intensity,
        params.environment_rotation_degrees.x,
        params.environment_rotation_degrees.y,
        params.environment_rotation_degrees.z,
        params.environment_intensity,
    ];
    let colors = [
        params.base_color_linear,
        params.environment_solid_color_linear,
        params.toon_shadow_color_linear,
        params.toon_rim_color_linear,
        params.toon_outline_color_linear,
        params.ground_toon_shadow_color_linear,
    ];
    if values.iter().any(|value| !value.is_finite())
        || colors.iter().any(|color| !color.is_finite())
        || params.point_lights.iter().any(|light| {
            !light.position.is_finite()
                || !light.intensity.is_finite()
                || !light.range.is_finite()
                || !light.radius.is_finite()
                || !light.color_linear.is_finite()
        })
        || params.sun_lights.iter().any(|light| {
            !light.rotation_degrees.is_finite()
                || !light.intensity.is_finite()
                || !light.angular_radius_degrees.is_finite()
                || !light.color_linear.is_finite()
        })
        || params.grounds.iter().any(|ground| {
            !ground.size.is_finite()
                || !ground.intensity.is_finite()
                || !ground.position.is_finite()
                || !ground.rotation_degrees.is_finite()
                || !ground.opacity.is_finite()
                || !ground.shadow_strength.is_finite()
                || !ground.reflection.is_finite()
                || !ground.roughness.is_finite()
        })
    {
        return Err(Render3dError::message(
            "scene contains a non-finite evaluated value",
        ));
    }
    if params.model_scale.abs().min_element() <= f32::EPSILON {
        return Err(Render3dError::message("model scale must be nonzero"));
    }
    Ok(())
}
