mod bvh;
pub use bvh::{SoftwareBvhNode, software_bvh};
use glam::{Mat4, Vec3, camera::rh::proj::directx};

use crate::{
    GroundShape, MAX_SCENE_GROUNDS, MAX_SCENE_LIGHTS, Render3dError, SceneProjection,
    SceneRenderParams, obj::SceneUniforms, validate_scene,
};

const AUTOMATIC_CLIP_MARGIN: f32 = 1.1;
const MIN_NEAR_PLANE: f32 = 0.001;
const MIN_ORTHOGRAPHIC_HEIGHT: f32 = 0.001;
const MIN_POINT_LIGHT_RANGE: f32 = 0.001;
const MIN_VERTICAL_FOV_DEGREES: f32 = 1.0;
const MAX_VERTICAL_FOV_DEGREES: f32 = 179.0;
const MAX_CAMERA_DISTANCE: f32 = 1_000_000.0;

pub fn resolve_scene_uniforms(
    width: u32,
    height: u32,
    params: &SceneRenderParams,
) -> Result<SceneUniforms, Render3dError> {
    validate_scene(width, height, params)?;

    let model = Mat4::from_scale_rotation_translation(
        params.model_scale,
        shrimply_transform_3d::rotation(params.model_rotation_degrees, params.model_rotation_order),
        params.model_position,
    ) * Mat4::from_translation(-params.model_anchor);
    let model_inverse = model.inverse();
    if !model.is_finite() || !model_inverse.is_finite() {
        return Err(Render3dError::message(
            "scene model transform exceeds the renderable range",
        ));
    }
    let normal = model_inverse.transpose();
    let camera_world =
        shrimply_transform_3d::camera_world(params.camera_position, params.camera_rotation_degrees);
    let view = camera_world.inverse();
    let center_view = view.transform_point3(model.transform_point3(Vec3::ZERO));
    let radius = params.model_scale.abs().max_element();
    let near = (-center_view.z - radius * AUTOMATIC_CLIP_MARGIN).max(MIN_NEAR_PLANE);
    let far = (-center_view.z + radius * AUTOMATIC_CLIP_MARGIN).max(near + MIN_NEAR_PLANE);
    let aspect = width as f32 / height as f32;
    let vertical_fov_degrees = params
        .vertical_fov_degrees
        .clamp(MIN_VERTICAL_FOV_DEGREES, MAX_VERTICAL_FOV_DEGREES);
    let projection = match params.camera_projection {
        SceneProjection::Perspective => {
            directx::perspective(vertical_fov_degrees.to_radians(), aspect, near, far)
        }
        SceneProjection::Orthographic => {
            let half_height = params
                .orthographic_height
                .clamp(MIN_ORTHOGRAPHIC_HEIGHT, MAX_CAMERA_DISTANCE)
                * 0.5;
            directx::orthographic(
                -half_height * aspect,
                half_height * aspect,
                -half_height,
                half_height,
                near,
                far,
            )
        }
        SceneProjection::Equirectangular
        | SceneProjection::Cylindrical
        | SceneProjection::Fisheye => directx::perspective(90.0f32.to_radians(), aspect, near, far),
    };
    let environment_inverse_rotation = Mat4::from_quat(
        shrimply_transform_3d::rotation(
            params.environment_rotation_degrees,
            shrimply_scene_3d::RotationOrder::Xyz,
        )
        .inverse(),
    );
    let mut point_lights: [crate::obj::PointLight; MAX_SCENE_LIGHTS] =
        std::array::from_fn(|_| crate::obj::PointLight::default());
    for (output, light) in point_lights.iter_mut().zip(&params.point_lights) {
        *output = crate::obj::PointLight {
            position_radius: light.position.extend(light.radius.max(0.0)).to_array(),
            color_intensity: [
                light.color_linear.r,
                light.color_linear.g,
                light.color_linear.b,
                light.intensity.max(0.0),
            ],
            range_padding: [light.range.max(MIN_POINT_LIGHT_RANGE), 0.0, 0.0, 0.0],
        };
    }
    let mut sun_lights: [crate::obj::SunLight; MAX_SCENE_LIGHTS] =
        std::array::from_fn(|_| crate::obj::SunLight::default());
    for (output, light) in sun_lights.iter_mut().zip(&params.sun_lights) {
        let direction = shrimply_transform_3d::rotation(
            light.rotation_degrees,
            shrimply_scene_3d::RotationOrder::Xyz,
        ) * Vec3::NEG_Z;
        *output = crate::obj::SunLight {
            direction_angular_radius: direction
                .extend(light.angular_radius_degrees.clamp(0.0, 45.0).to_radians())
                .to_array(),
            color_intensity: [
                light.color_linear.r,
                light.color_linear.g,
                light.color_linear.b,
                light.intensity.max(0.0),
            ],
        };
    }
    let mut grounds: [crate::obj::Ground; MAX_SCENE_GROUNDS] =
        std::array::from_fn(|_| crate::obj::Ground::default());
    for (output, ground) in grounds.iter_mut().zip(&params.grounds) {
        let rotation = shrimply_transform_3d::rotation(
            ground.rotation_degrees,
            shrimply_scene_3d::RotationOrder::Xyz,
        );
        *output = crate::obj::Ground {
            position_opacity: ground
                .position
                .extend(ground.opacity.clamp(0.0, 1.0))
                .to_array(),
            basis_x_size: (rotation * Vec3::X)
                .extend(ground.size.max(f32::EPSILON))
                .to_array(),
            normal_composite: (rotation * Vec3::Y)
                .extend(u32::from(ground.composite_enabled) as f32)
                .to_array(),
            basis_z_intensity: (rotation * Vec3::Z)
                .extend(ground.intensity.max(0.0))
                .to_array(),
            settings: [
                ground.reflection.clamp(0.0, 1.0),
                ground.shadow_strength.clamp(0.0, 1.0),
                ground.roughness.clamp(0.0, 1.0),
                match ground.shape {
                    GroundShape::Infinite => 0.0,
                    GroundShape::Square => 1.0,
                },
            ],
        };
    }
    let shadow_receiver_normal = shrimply_transform_3d::rotation(
        params.shadow_receiver_rotation_degrees,
        shrimply_scene_3d::RotationOrder::Xyz,
    ) * Vec3::Y;

    let view_projection = projection * view;
    let inverse_view_projection = view_projection.inverse();
    if !view_projection.is_finite() || !inverse_view_projection.is_finite() {
        return Err(Render3dError::message(
            "scene camera transform exceeds the renderable range",
        ));
    }
    let aperture_radius = match params.camera_projection {
        SceneProjection::Perspective if params.focus_distance > 0.0 => {
            (shrimply_transform_3d::focal_length_mm(vertical_fov_degrees as f64) as f32 / 1_000.0)
                / (2.0
                    * params
                        .f_stop
                        .clamp(shrimply_scene_3d::MIN_F_STOP, shrimply_scene_3d::MAX_F_STOP))
        }
        _ => 0.0,
    };
    Ok(SceneUniforms {
        model: model.to_cols_array(),
        model_inverse: model_inverse.to_cols_array(),
        normal: normal.to_cols_array(),
        camera_world: camera_world.to_cols_array(),
        view_projection: view_projection.to_cols_array(),
        inverse_view_projection: inverse_view_projection.to_cols_array(),
        environment_inverse_rotation: environment_inverse_rotation.to_cols_array(),
        camera_position: params.camera_position.extend(1.0).to_array(),
        camera_lens: [
            params
                .focus_distance
                .clamp(MIN_NEAR_PLANE, MAX_CAMERA_DISTANCE),
            aperture_radius.max(0.0),
            match params.camera_projection {
                SceneProjection::Fisheye => {
                    params
                        .vertical_fov_degrees
                        .clamp(MIN_VERTICAL_FOV_DEGREES, 360.0)
                        .to_radians()
                        * 0.5
                }
                _ => (vertical_fov_degrees.to_radians() * 0.5).tan(),
            },
            match params.camera_projection {
                SceneProjection::Perspective => 0.0,
                SceneProjection::Orthographic => 1.0,
                SceneProjection::Equirectangular => 2.0,
                SceneProjection::Cylindrical => 3.0,
                SceneProjection::Fisheye => 4.0,
            },
        ],
        base_color: params.base_color_linear.to_array(),
        point_lights,
        sun_lights,
        grounds,
        shadow_receiver_position_opacity: params
            .shadow_receiver_position
            .extend(params.shadow_receiver_opacity.clamp(0.0, 1.0))
            .to_array(),
        shadow_receiver_normal_enabled: shadow_receiver_normal
            .extend(u32::from(params.shadow_receiver_enabled) as f32)
            .to_array(),
        ground_settings: [
            params.ground_reflection.clamp(0.0, 1.0),
            params.ground_shadow_strength.clamp(0.0, 1.0),
            params.ground_roughness.clamp(0.0, 1.0),
            u32::from(params.ground_composite_enabled) as f32,
        ],
        transmission_background: [
            params
                .background_distance
                .max(shrimply_scene_3d::MIN_NEAR_PLANE),
            0.0,
            match params.background_address_mode {
                shrimply_scene_3d::BackgroundAddressMode::ExtendEdge => 0.0,
                shrimply_scene_3d::BackgroundAddressMode::Repeat => 1.0,
                shrimply_scene_3d::BackgroundAddressMode::Mirror => 2.0,
            },
            u32::from(params.background_plane_enabled) as f32,
        ],
        composed_plane_intensity: [
            params.ground_intensity.max(0.0),
            params.background_intensity.max(0.0),
            0.0,
            0.0,
        ],
        material: [
            params.metallic.clamp(0.0, 1.0),
            params
                .roughness
                .clamp(shrimply_scene_3d::MIN_ROUGHNESS, 1.0),
            2.0f32.powf(params.exposure_ev.clamp(
                shrimply_scene_3d::MIN_EXPOSURE_EV,
                shrimply_scene_3d::MAX_EXPOSURE_EV,
            )),
            0.0,
        ],
        environment_color: [
            params.environment_solid_color_linear.r,
            params.environment_solid_color_linear.g,
            params.environment_solid_color_linear.b,
            1.0,
        ],
        point_light_count: params.point_lights.len() as u32,
        sun_light_count: params.sun_lights.len() as u32,
        ground_count: params.grounds.len() as u32,
        _ground_count_padding: 0,
        ground_toon_shadow: [
            params.ground_toon_shadow_color_linear.r,
            params.ground_toon_shadow_color_linear.g,
            params.ground_toon_shadow_color_linear.b,
            params.ground_toon_shadow_strength.clamp(0.0, 1.0),
        ],
        ground_shading_model: params.ground_shading_model,
        ground_toon_shadow_kind: params.ground_toon_shadow_kind,
        normal_mode: params.normal_mode,
        _ground_style_padding: 0,
        environment: crate::obj::EnvironmentSettings {
            intensity: params.environment_intensity.max(0.0),
            source: match params.environment_source {
                shrimply_scene_3d::EnvironmentSource::Composite => {
                    crate::obj::EnvironmentSource::Composite
                }
                shrimply_scene_3d::EnvironmentSource::Image => crate::obj::EnvironmentSource::Image,
                shrimply_scene_3d::EnvironmentSource::Black => crate::obj::EnvironmentSource::Black,
            },
            _padding: [0.0; 2],
        },
        pbr: pbr_settings(params),
        toon: toon_settings(params),
    })
}

pub(crate) fn pbr_settings(params: &SceneRenderParams) -> crate::obj::PbrSettings {
    let mut settings = crate::obj::PbrSettings::default();
    let pbr = params.shading_model == crate::obj::ShadingModel::Pbr;
    settings.path_tracing = if pbr {
        params.path_tracing
    } else {
        crate::obj::PathTracingMode::Off
    };
    settings.light_sampling_quality = params.light_sampling_quality;
    settings.render_quality = params.render_quality;
    settings.denoising = u32::from(params.denoising && pbr);
    settings.subsurface = params.subsurface.clamp(0.0, 1.0);
    settings.clearcoat = params.clearcoat.clamp(0.0, 1.0);
    settings.sheen = params.sheen.clamp(0.0, 1.0);
    settings.transmission = if pbr {
        params.transmission.clamp(0.0, 1.0)
    } else {
        0.0
    };
    settings.ior = params
        .ior
        .clamp(shrimply_scene_3d::MIN_IOR, shrimply_scene_3d::MAX_IOR);
    settings
}

pub(crate) fn toon_settings(params: &SceneRenderParams) -> crate::obj::ToonSettings {
    crate::obj::ToonSettings {
        shadow: [
            params.toon_shadow_color_linear.r,
            params.toon_shadow_color_linear.g,
            params.toon_shadow_color_linear.b,
            params.toon_shadow_strength.clamp(0.0, 1.0),
        ],
        rim: [
            params.toon_rim_color_linear.r,
            params.toon_rim_color_linear.g,
            params.toon_rim_color_linear.b,
            params.toon_rim_strength.max(0.0),
        ],
        shading_model: params.shading_model,
        bands: params.toon_bands.round().clamp(2.0, 16.0),
        texture_filter: params.toon_texture_filter,
        rim_power: params.toon_rim_power.max(MIN_NEAR_PLANE),
        specular_size: params.toon_specular_size.clamp(0.0, 1.0),
        specular_strength: params.toon_specular_strength.max(0.0),
        color_levels: params.toon_color_levels.round().clamp(2.0, 32.0),
        kuwahara_radius: params.toon_kuwahara_radius.round().clamp(0.0, 4.0),
        kuwahara_strength: params.toon_kuwahara_strength.clamp(0.0, 1.0),
        shadow_kind: params.toon_shadow_kind,
        shadow_darkest_tone: params
            .toon_shadow_darkest_tone
            .clamp(shrimply_scene_3d::MIN_TOON_SHADOW_TONE, 1.0),
        shadow_crosshatch_max_directions: params
            .toon_shadow_crosshatch_max_directions
            .round()
            .clamp(1.0, 6.0),
        shadow_dot_size: params.toon_shadow_dot_size.clamp(0.25, 64.0),
        shadow_dot_density: params.toon_shadow_dot_density.clamp(0.001, 1.0),
        shadow_dot_distribution_randomness: params
            .toon_shadow_dot_distribution_randomness
            .clamp(0.0, 1.0),
        shadow_dot_size_randomness: params.toon_shadow_dot_size_randomness.clamp(0.0, 1.0),
        shadow_line_direction: params
            .toon_shadow_line_direction_degrees
            .rem_euclid(180.0)
            .to_radians(),
        shadow_line_width: params.toon_shadow_line_width.clamp(0.25, 64.0),
        shadow_line_density: params.toon_shadow_line_density.clamp(0.001, 1.0),
        shadow_line_distribution_randomness: params
            .toon_shadow_line_distribution_randomness
            .clamp(0.0, 1.0),
        shadow_line_width_randomness: params.toon_shadow_line_width_randomness.clamp(0.0, 1.0),
        shadow_pattern_softness: params.toon_shadow_pattern_softness.clamp(0.0, 4.0),
        shadow_crosshatch_angle: params
            .toon_shadow_crosshatch_angle_degrees
            .rem_euclid(180.0)
            .to_radians(),
        _shadow_quantization_padding: [0.0; 2],
        outline_frequency: [
            params.toon_outline_offset_frequency.clamp(0.001, 1.0),
            params.toon_outline_width_frequency.clamp(0.001, 1.0),
        ],
        outline_color: params.toon_outline_color_linear.to_array(),
        outline_mode: params.toon_outline_mode,
        outline_width: params.toon_outline_width.clamp(0.25, 16.0),
        outline_opacity: params.toon_outline_opacity.clamp(0.0, 1.0),
        anti_aliasing: params.anti_aliasing,
        outline_quality: params.toon_outline_quality,
        outline_depth_threshold: params.toon_outline_depth_threshold.clamp(0.0, 1.0),
        outline_normal_cosine: params
            .toon_outline_normal_angle_degrees
            .clamp(0.0, 180.0)
            .to_radians()
            .cos(),
        outline_method: params.toon_outline_method,
        dog_inner_radius: params.toon_outline_dog_inner_radius.clamp(0.25, 16.0),
        dog_radius_ratio: params.toon_outline_dog_radius_ratio.clamp(1.01, 4.0),
        dog_threshold: params.toon_outline_dog_threshold.clamp(0.0, 0.25),
        dog_sharpness: params.toon_outline_dog_sharpness.clamp(1.0, 64.0),
        outline_offset_variation: params.toon_outline_offset_variation.clamp(0.0, 16.0),
        outline_width_variation: params.toon_outline_width_variation.clamp(0.0, 16.0),
        outline_noise_seed: params.toon_outline_noise_seed,
        outline_noise_evolution: params.toon_outline_noise_evolution,
        outline_aggressiveness: params.toon_outline_aggressiveness.clamp(0.1, 8.0),
    }
}
