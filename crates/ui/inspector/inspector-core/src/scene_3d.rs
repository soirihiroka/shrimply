use ffmpeg_next as ffmpeg;
use serde::Serialize;
use shrimply_project_document::project::{Asset, ItemAddress, Project, Time};
use shrimply_property_model::{Color, timeline_value::TimelineValue};
use shrimply_scene_3d::{
    AntiAliasing, BackgroundAddressMode, Camera3d, CameraProjection, Environment3d,
    EnvironmentSource, LightSamplingQuality, ObjScene, PathTracingMode, PbrMaterial, ShadingModel,
    ToonOutlineMethod, ToonOutlineMode, ToonOutlineQuality, ToonShadowKind, ToonTextureFilter,
};

use crate::{
    ControlKind, InspectorControl, InspectorControlAction, InspectorController, InspectorRuntime,
    InspectorSection, InspectorTarget, LayeredState, NumberMapping, NumberSpec, VideoCard,
    VideoReset,
};

pub const CAMERA_PROJECTION_PATH: &str = "/content/camera/projection";
pub const CAMERA_ANTI_ALIASING_PATH: &str = "/content/camera/anti_aliasing";
pub const CAMERA_POSITION_PATH: &str = "/content/camera/position";
pub const CAMERA_ROTATION_PATH: &str = "/content/camera/rotation_degrees";
pub const CAMERA_FOV_PATH: &str = "/content/camera/vertical_fov_degrees";
pub const CAMERA_FOCUS_DISTANCE_PATH: &str = "/content/camera/focus_distance";
pub const CAMERA_F_STOP_PATH: &str = "/content/camera/f_stop";
pub const CAMERA_ORTHOGRAPHIC_HEIGHT_PATH: &str = "/content/camera/orthographic_height";
pub const CAMERA_BACKGROUND_ENABLED_PATH: &str = "/content/camera/background_plane_enabled";
pub const CAMERA_BACKGROUND_DISTANCE_PATH: &str = "/content/camera/background_distance";
pub const CAMERA_BACKGROUND_INTENSITY_PATH: &str = "/content/camera/background_intensity";
pub const CAMERA_BACKGROUND_ADDRESS_PATH: &str = "/content/camera/background_address_mode";
pub const CAMERA_EXPOSURE_PATH: &str = "/content/camera/exposure_ev";

pub const MATERIAL_PATH: &str = "/content/material";
pub const SHADING_PATH: &str = "/content/material/shading_model";
pub const OUTLINE_MODE_PATH: &str = "/content/material/toon/outline/mode";
pub const OUTLINE_METHOD_PATH: &str = "/content/material/toon/outline/method";
pub const OUTLINE_QUALITY_PATH: &str = "/content/material/toon/outline/quality";
pub const TEXTURE_FILTER_PATH: &str = "/content/material/toon/texture_filter";
pub const SHADOW_KIND_PATH: &str = "/content/material/toon/shadow_kind";
pub const PATH_TRACING_PATH: &str = "/content/material/path_tracing";
pub const LIGHT_SAMPLING_PATH: &str = "/content/material/light_sampling_quality";
pub const DENOISING_PATH: &str = "/content/material/denoising";

pub const ENVIRONMENT_SOURCE_PATH: &str = "/content/environment/source";
pub const ENVIRONMENT_FILE_PATH: &str = "/content/environment/file";
pub const ENVIRONMENT_ROTATION_PATH: &str = "/content/environment/rotation_degrees";
pub const ENVIRONMENT_COLOR_PATH: &str = "/content/environment/solid_color";
pub const ENVIRONMENT_INTENSITY_PATH: &str = "/content/environment/intensity";

pub const VECTOR_COMMIT: &str = "edit-scene-3d-vec3";
pub const VECTOR_EXPRESSION_COMMIT: &str = "edit-scene-3d-vec3-expression";
pub const SCALAR_COMMIT: &str = "edit-scene-3d-scalar";
pub const COLOR_COMMIT: &str = "edit-scene-3d-color";

pub fn cards(
    project: &Project,
    address: &ItemAddress,
    scene: &ObjScene,
    runtime: InspectorRuntime,
    camera_models: Option<&Result<Vec<String>, String>>,
) -> [VideoCard; 3] {
    [
        camera_card(project, address, &scene.camera, runtime, camera_models),
        render_card(&scene.material, runtime),
        environment_card(&scene.environment, runtime),
    ]
}

pub fn camera_card(
    project: &Project,
    address: &ItemAddress,
    camera: &Camera3d,
    runtime: InspectorRuntime,
    camera_models: Option<&Result<Vec<String>, String>>,
) -> VideoCard {
    let source =
        crate::camera_source::presentation(project, address, &camera.source, camera_models);
    let custom = source.custom;
    let mut section = source.section;
    if custom {
        section.add(selector(
            CAMERA_PROJECTION_PATH,
            "Projection",
            camera.projection,
            &[
                (CameraProjection::Perspective, "Perspective"),
                (CameraProjection::Orthographic, "Orthographic"),
                (CameraProjection::Equirectangular, "Equirectangular"),
                (CameraProjection::Cylindrical, "Cylindrical"),
                (CameraProjection::Fisheye, "Fisheye"),
            ],
            "edit-scene-3d-projection",
        ));
        section.add(selector(
            CAMERA_ANTI_ALIASING_PATH,
            "Anti-aliasing",
            camera.anti_aliasing,
            &[
                (AntiAliasing::None, "Off"),
                (AntiAliasing::RotatedGrid2x, "2× rotated grid"),
                (AntiAliasing::Grid4x, "4× grid"),
                (AntiAliasing::Stochastic8x, "8× stochastic"),
            ],
            "edit-scene-3d-anti-aliasing",
        ));
    }
    section.add(vector(
        CAMERA_POSITION_PATH,
        "Position",
        &camera.position,
        runtime,
        false,
    ));
    section.add(vector(
        CAMERA_ROTATION_PATH,
        "Rotation",
        &camera.rotation_degrees,
        runtime,
        true,
    ));
    if custom {
        match camera.projection {
            CameraProjection::Perspective => {
                section.add(scalar(
                    CAMERA_FOV_PATH,
                    "Focal length",
                    &camera.vertical_fov_degrees,
                    runtime,
                    ScalarKind::FocalLength,
                ));
                section.add(scalar(
                    CAMERA_FOCUS_DISTANCE_PATH,
                    "Focus distance (0 = off)",
                    &camera.focus_distance,
                    runtime,
                    ScalarKind::Nonnegative,
                ));
                section.add(scalar(
                    CAMERA_F_STOP_PATH,
                    "Aperture",
                    &camera.f_stop,
                    runtime,
                    ScalarKind::FStop,
                ));
            }
            CameraProjection::Orthographic => section.add(scalar(
                CAMERA_ORTHOGRAPHIC_HEIGHT_PATH,
                "Orthographic height",
                &camera.orthographic_height,
                runtime,
                ScalarKind::Positive,
            )),
            CameraProjection::Cylindrical => section.add(scalar(
                CAMERA_FOV_PATH,
                "Vertical FOV",
                &camera.vertical_fov_degrees,
                runtime,
                ScalarKind::Fov,
            )),
            CameraProjection::Equirectangular => {}
            CameraProjection::Fisheye => section.add(scalar(
                CAMERA_FOV_PATH,
                "FOV",
                &camera.vertical_fov_degrees,
                runtime,
                ScalarKind::FisheyeFov,
            )),
        }
        section.add(selector(
            CAMERA_BACKGROUND_ENABLED_PATH,
            "Background plane",
            camera.background_plane_enabled,
            &[(false, "Off"), (true, "On")],
            "edit-scene-3d-background-plane",
        ));
        if camera.background_plane_enabled {
            section.add(scalar(
                CAMERA_BACKGROUND_DISTANCE_PATH,
                "Background distance",
                &camera.background_distance,
                runtime,
                ScalarKind::Positive,
            ));
            section.add(scalar(
                CAMERA_BACKGROUND_INTENSITY_PATH,
                "Background intensity",
                &camera.background_intensity,
                runtime,
                ScalarKind::Nonnegative,
            ));
            section.add(selector(
                CAMERA_BACKGROUND_ADDRESS_PATH,
                "Background tiling",
                camera.background_address_mode,
                &[
                    (BackgroundAddressMode::ExtendEdge, "Extend edge"),
                    (BackgroundAddressMode::Repeat, "Repeat"),
                    (BackgroundAddressMode::Mirror, "Mirror"),
                ],
                "edit-scene-3d-background-addressing",
            ));
        }
        section.add(scalar(
            CAMERA_EXPOSURE_PATH,
            "Exposure",
            &camera.exposure_ev,
            runtime,
            ScalarKind::Exposure,
        ));
    }
    card(
        "scene-3d-camera",
        "Camera",
        section,
        "/content/camera",
        &Camera3d::default(),
        "reset-scene-3d-camera",
    )
}

pub fn render_card(material: &PbrMaterial, runtime: InspectorRuntime) -> VideoCard {
    let mut section = InspectorSection::default();
    section.add(selector(
        SHADING_PATH,
        "Shader",
        material.shading_model,
        &[
            (ShadingModel::Pbr, "PBR"),
            (ShadingModel::Toon, "Toon"),
            (ShadingModel::Depth, "Depth"),
        ],
        "edit-scene-3d-shading-model",
    ));
    add_outline(&mut section, material, runtime);
    match material.shading_model {
        ShadingModel::Pbr => {
            section.add(selector(
                PATH_TRACING_PATH,
                "Path tracing",
                material.path_tracing,
                &[
                    (PathTracingMode::Off, "Off"),
                    (PathTracingMode::Samples1, "1"),
                    (PathTracingMode::Samples2, "2"),
                    (PathTracingMode::Preview, "4"),
                    (PathTracingMode::Samples8, "8"),
                    (PathTracingMode::Quality, "16"),
                    (PathTracingMode::Samples32, "32"),
                    (PathTracingMode::Samples64, "64"),
                    (PathTracingMode::Samples128, "128"),
                ],
                "edit-scene-3d-path-tracing",
            ));
            section.add(light_sampling(material.light_sampling_quality));
            section.add(selector(
                DENOISING_PATH,
                "Denoiser",
                material.denoising,
                &denoising_methods(material.denoising),
                "edit-scene-3d-denoising",
            ));
        }
        ShadingModel::Toon => add_toon(&mut section, material, runtime),
        ShadingModel::Depth => {}
    }
    card(
        "scene-3d-render",
        "Render Style",
        section,
        MATERIAL_PATH,
        &PbrMaterial::default(),
        "reset-scene-3d-render",
    )
}

fn denoising_methods(
    selected: shrimply_scene_3d::DenoisingMethod,
) -> Vec<(shrimply_scene_3d::DenoisingMethod, &'static str)> {
    #[cfg(feature = "cuda-backend")]
    let mut methods = shrimply_visual_cuda::gpu::denoising_methods();
    #[cfg(not(feature = "cuda-backend"))]
    let mut methods = vec![
        (shrimply_scene_3d::DenoisingMethod::Auto, "Auto"),
        (shrimply_scene_3d::DenoisingMethod::Off, "Off"),
    ];
    #[cfg(all(not(feature = "cuda-backend"), target_os = "macos"))]
    methods.push((shrimply_scene_3d::DenoisingMethod::MetalSvgf, "Metal SVGF"));
    if !methods.iter().any(|(method, _)| *method == selected) {
        methods.push((selected, "Unavailable on this device"));
    }
    methods
}

fn add_toon(section: &mut InspectorSection, material: &PbrMaterial, runtime: InspectorRuntime) {
    let toon = &material.toon;
    section.add(light_sampling(material.light_sampling_quality));
    add_number(
        section,
        "/content/material/toon/bands",
        "Light bands",
        &toon.bands,
        runtime,
        ScalarKind::Bands,
    );
    add_number(
        section,
        "/content/material/toon/color_levels",
        "Palette levels",
        &toon.color_levels,
        runtime,
        ScalarKind::ColorLevels,
    );
    section.add(selector(
        TEXTURE_FILTER_PATH,
        "Texture filter",
        toon.texture_filter,
        &[
            (ToonTextureFilter::Direct, "Direct"),
            (ToonTextureFilter::Kuwahara, "Kuwahara · painterly"),
        ],
        "edit-scene-3d-texture-filter",
    ));
    if toon.texture_filter == ToonTextureFilter::Kuwahara {
        add_number(
            section,
            "/content/material/toon/kuwahara_radius",
            "Kuwahara radius",
            &toon.kuwahara_radius,
            runtime,
            ScalarKind::KuwaharaRadius,
        );
        add_number(
            section,
            "/content/material/toon/kuwahara_strength",
            "Kuwahara strength",
            &toon.kuwahara_strength,
            runtime,
            ScalarKind::Unit,
        );
    }
    section.add(selector(
        SHADOW_KIND_PATH,
        "Shadow kind",
        toon.shadow_kind,
        &[
            (ToonShadowKind::Solid, "Solid"),
            (ToonShadowKind::Dots, "Dots"),
            (ToonShadowKind::Lines, "Lines"),
            (ToonShadowKind::Crosshatch, "Crosshatch"),
        ],
        "edit-scene-3d-shadow-kind",
    ));
    section.add(color_control(
        "/content/material/toon/shadow_color",
        "Shadow color",
        &toon.shadow_color,
        runtime,
    ));
    add_number(
        section,
        "/content/material/toon/shadow_strength",
        "Shadow strength",
        &toon.shadow_strength,
        runtime,
        ScalarKind::Unit,
    );
    add_number(
        section,
        "/content/material/toon/shadow_darkest_tone",
        "Darkest tone",
        &toon.shadow_darkest_tone,
        runtime,
        ScalarKind::ShadowTone,
    );
    match toon.shadow_kind {
        ToonShadowKind::Solid => {}
        ToonShadowKind::Dots => {
            add_number(
                section,
                "/content/material/toon/shadow_dot_size",
                "Dot size",
                &toon.shadow_dot_size,
                runtime,
                ScalarKind::ShadowPatternSize,
            );
            add_number(
                section,
                "/content/material/toon/shadow_dot_density",
                "Dot density",
                &toon.shadow_dot_density,
                runtime,
                ScalarKind::Frequency,
            );
            add_number(
                section,
                "/content/material/toon/shadow_dot_distribution_randomness",
                "Distribution randomness",
                &toon.shadow_dot_distribution_randomness,
                runtime,
                ScalarKind::Unit,
            );
            add_number(
                section,
                "/content/material/toon/shadow_dot_size_randomness",
                "Size randomness",
                &toon.shadow_dot_size_randomness,
                runtime,
                ScalarKind::Unit,
            );
        }
        ToonShadowKind::Lines | ToonShadowKind::Crosshatch => {
            add_number(
                section,
                "/content/material/toon/shadow_line_direction_degrees",
                "Line direction",
                &toon.shadow_line_direction_degrees,
                runtime,
                ScalarKind::ClampedDegrees,
            );
            add_number(
                section,
                "/content/material/toon/shadow_line_width",
                "Line width",
                &toon.shadow_line_width,
                runtime,
                ScalarKind::ShadowPatternSize,
            );
            add_number(
                section,
                "/content/material/toon/shadow_line_density",
                "Line density",
                &toon.shadow_line_density,
                runtime,
                ScalarKind::Frequency,
            );
            add_number(
                section,
                "/content/material/toon/shadow_line_distribution_randomness",
                "Distribution randomness",
                &toon.shadow_line_distribution_randomness,
                runtime,
                ScalarKind::Unit,
            );
            add_number(
                section,
                "/content/material/toon/shadow_line_width_randomness",
                "Width randomness",
                &toon.shadow_line_width_randomness,
                runtime,
                ScalarKind::Unit,
            );
            if toon.shadow_kind == ToonShadowKind::Crosshatch {
                add_number(
                    section,
                    "/content/material/toon/shadow_crosshatch_max_directions",
                    "Maximum directions",
                    &toon.shadow_crosshatch_max_directions,
                    runtime,
                    ScalarKind::DirectionCount,
                );
            }
        }
    }
    if toon.shadow_kind != ToonShadowKind::Solid {
        add_number(
            section,
            "/content/material/toon/shadow_pattern_softness",
            "Pattern softness",
            &toon.shadow_pattern_softness,
            runtime,
            ScalarKind::ShadowSoftness,
        );
    }
    section.add(color_control(
        "/content/material/toon/rim_color",
        "Rim tint",
        &toon.rim_color,
        runtime,
    ));
    add_number(
        section,
        "/content/material/toon/rim_strength",
        "Rim strength",
        &toon.rim_strength,
        runtime,
        ScalarKind::Nonnegative,
    );
    add_number(
        section,
        "/content/material/toon/rim_power",
        "Rim power",
        &toon.rim_power,
        runtime,
        ScalarKind::Positive,
    );
    add_number(
        section,
        "/content/material/toon/specular_size",
        "Specular size",
        &toon.specular_size,
        runtime,
        ScalarKind::Unit,
    );
    add_number(
        section,
        "/content/material/toon/specular_strength",
        "Specular strength",
        &toon.specular_strength,
        runtime,
        ScalarKind::Nonnegative,
    );
}

fn add_outline(section: &mut InspectorSection, material: &PbrMaterial, runtime: InspectorRuntime) {
    let outline = &material.toon.outline;
    section.add(selector(
        OUTLINE_MODE_PATH,
        "Outline",
        outline.mode,
        &[
            (ToonOutlineMode::Off, "Off"),
            (ToonOutlineMode::Silhouette, "Silhouette"),
            (
                ToonOutlineMode::SilhouetteAndCreases,
                "Silhouette + creases",
            ),
        ],
        "edit-scene-3d-outline-mode",
    ));
    if outline.mode == ToonOutlineMode::Off {
        return;
    }
    section.add(selector(
        OUTLINE_METHOD_PATH,
        "Outline method",
        outline.method,
        &[
            (ToonOutlineMethod::RayTraced, "Radial probes"),
            (ToonOutlineMethod::Fresnel, "Fresnel contour"),
            (ToonOutlineMethod::Hybrid, "Radial + Fresnel"),
            (ToonOutlineMethod::Sobel, "Sobel"),
            (ToonOutlineMethod::RobertsCross, "Roberts cross"),
            (ToonOutlineMethod::DifferenceOfGaussians, "XDoG"),
            (
                ToonOutlineMethod::RegionBoundary,
                "Feature contours · multipass",
            ),
        ],
        "edit-scene-3d-outline-method",
    ));
    if matches!(
        outline.method,
        ToonOutlineMethod::RayTraced
            | ToonOutlineMethod::Hybrid
            | ToonOutlineMethod::DifferenceOfGaussians
    ) {
        let labels = if outline.method == ToonOutlineMethod::DifferenceOfGaussians {
            [
                "Standard · 12 samples",
                "High · 24 samples",
                "Ultra · 48 samples",
            ]
        } else {
            [
                "Standard · 4 probes",
                "High · 8 probes",
                "Ultra · 16 probes",
            ]
        };
        section.add(selector(
            OUTLINE_QUALITY_PATH,
            "Outline quality",
            outline.quality,
            &[
                (ToonOutlineQuality::Standard, labels[0]),
                (ToonOutlineQuality::High, labels[1]),
                (ToonOutlineQuality::Ultra, labels[2]),
            ],
            "edit-scene-3d-outline-quality",
        ));
    }
    section.add(color_control(
        "/content/material/toon/outline/color",
        "Default outline color",
        &outline.color,
        runtime,
    ));
    if outline.method == ToonOutlineMethod::DifferenceOfGaussians {
        add_number(
            section,
            "/content/material/toon/outline/dog_inner_radius",
            "Sigma",
            &outline.dog_inner_radius,
            runtime,
            ScalarKind::OutlineWidth,
        );
        add_number(
            section,
            "/content/material/toon/outline/dog_radius_ratio",
            "Sigma ratio",
            &outline.dog_radius_ratio,
            runtime,
            ScalarKind::RadiusRatio,
        );
        add_number(
            section,
            "/content/material/toon/outline/dog_threshold",
            "Sensitivity",
            &outline.dog_threshold,
            runtime,
            ScalarKind::DogSensitivity,
        );
        add_number(
            section,
            "/content/material/toon/outline/dog_sharpness",
            "Sharpness",
            &outline.dog_sharpness,
            runtime,
            ScalarKind::DogSharpness,
        );
    } else {
        add_number(
            section,
            "/content/material/toon/outline/width",
            "Outline width",
            &outline.width,
            runtime,
            ScalarKind::OutlineWidth,
        );
    }
    for (path, label, value, kind) in [
        (
            "/content/material/toon/outline/opacity",
            "Outline opacity",
            &outline.opacity,
            ScalarKind::Unit,
        ),
        (
            "/content/material/toon/outline/aggressiveness",
            "Aggressiveness",
            &outline.aggressiveness,
            ScalarKind::Aggressiveness,
        ),
        (
            "/content/material/toon/outline/offset_variation",
            "Offset variation",
            &outline.offset_variation,
            ScalarKind::PixelVariation,
        ),
        (
            "/content/material/toon/outline/offset_frequency",
            "Offset frequency",
            &outline.offset_frequency,
            ScalarKind::Frequency,
        ),
        (
            "/content/material/toon/outline/width_variation",
            "Width variation",
            &outline.width_variation,
            ScalarKind::PixelVariation,
        ),
        (
            "/content/material/toon/outline/width_frequency",
            "Width frequency",
            &outline.width_frequency,
            ScalarKind::Frequency,
        ),
        (
            "/content/material/toon/outline/noise_seed",
            "Noise seed",
            &outline.noise_seed,
            ScalarKind::Seed,
        ),
        (
            "/content/material/toon/outline/noise_evolution",
            "Noise evolution",
            &outline.noise_evolution,
            ScalarKind::Plain,
        ),
    ] {
        add_number(section, path, label, value, runtime, kind);
    }
    if outline.mode == ToonOutlineMode::SilhouetteAndCreases
        && outline.method != ToonOutlineMethod::Fresnel
    {
        add_number(
            section,
            "/content/material/toon/outline/depth_threshold",
            "Depth threshold",
            &outline.depth_threshold,
            runtime,
            ScalarKind::Unit,
        );
        add_number(
            section,
            "/content/material/toon/outline/normal_angle_degrees",
            "Normal angle",
            &outline.normal_angle_degrees,
            runtime,
            ScalarKind::ClampedDegrees,
        );
    }
}

pub fn environment_card(environment: &Environment3d, runtime: InspectorRuntime) -> VideoCard {
    let source = environment.effective_source();
    let mut section = InspectorSection::default();
    section.add(selector(
        ENVIRONMENT_SOURCE_PATH,
        "Source",
        source,
        &[
            (EnvironmentSource::Composite, "Composite"),
            (EnvironmentSource::Image, "HDRI"),
            (EnvironmentSource::Black, "Solid color"),
        ],
        "edit-scene-3d-environment-source",
    ));
    if source == EnvironmentSource::Image {
        let mut select = InspectorControl::new(
            ControlKind::Action,
            format!("{ENVIRONMENT_FILE_PATH}/select"),
            "Image",
        )
        .value(if environment.file.is_some() {
            "Replace image"
        } else {
            "Select image"
        })
        .action(InspectorControlAction::SelectScene3dEnvironment);
        select.prefix_icon = "folder-open-symbolic".to_string();
        section.add(select);
        let mut clear = InspectorControl::new(
            ControlKind::Action,
            format!("{ENVIRONMENT_FILE_PATH}/clear"),
            "",
        )
        .value("Clear image")
        .sensitive(environment.file.is_some())
        .action(InspectorControlAction::ClearScene3dEnvironment);
        clear.prefix_icon = "window-close-symbolic".to_string();
        section.add(clear);
    }
    if source == EnvironmentSource::Black {
        section.add(color_control(
            ENVIRONMENT_COLOR_PATH,
            "Color",
            &environment.solid_color,
            runtime,
        ));
    } else {
        section.add(vector(
            ENVIRONMENT_ROTATION_PATH,
            "Rotation",
            &environment.rotation_degrees,
            runtime,
            true,
        ));
    }
    section.add(scalar(
        ENVIRONMENT_INTENSITY_PATH,
        "Intensity",
        &environment.intensity,
        runtime,
        ScalarKind::Nonnegative,
    ));
    card(
        "scene-3d-environment",
        "Environment",
        section,
        "/content/environment",
        &Environment3d::default(),
        "reset-scene-3d-environment",
    )
}

fn card<T: Serialize>(
    key: &'static str,
    title: &'static str,
    section: InspectorSection,
    path: &str,
    default: &T,
    commit_name: &'static str,
) -> VideoCard {
    VideoCard {
        key,
        title,
        section,
        reset: Some(VideoReset {
            values: vec![(
                path.to_string(),
                serde_json::to_value(default).expect("scene 3D default must serialize"),
            )],
            fraction: None,
            commit_name,
            cancel_stabilization: false,
            paint_palette: false,
        }),
        alpha_mask: None,
        preview_facet: None,
        actions: Vec::new(),
        scope: crate::VideoCardScope::Source,
    }
}

fn selector<T: Copy + Serialize>(
    path: &'static str,
    label: &'static str,
    value: T,
    choices: &[(T, &'static str)],
    commit: &'static str,
) -> InspectorControl {
    crate::selector::selector(
        path,
        label,
        enum_text(value),
        choices
            .iter()
            .map(|(value, label)| (enum_text(*value), (*label).to_string())),
    )
    .immediate_commit(commit)
}

fn enum_text(value: impl Serialize) -> String {
    match serde_json::to_value(value).expect("scene 3D selector must serialize") {
        serde_json::Value::String(value) => value,
        serde_json::Value::Bool(value) => value.to_string(),
        value => panic!("scene 3D selector must serialize as text or boolean: {value}"),
    }
}

fn light_sampling(value: LightSamplingQuality) -> InspectorControl {
    selector(
        LIGHT_SAMPLING_PATH,
        "Shadow quality",
        value,
        &[
            (LightSamplingQuality::Rays1, "1"),
            (LightSamplingQuality::Rays2, "2"),
            (LightSamplingQuality::Standard, "4"),
            (LightSamplingQuality::High, "8"),
            (LightSamplingQuality::Ultra, "16"),
            (LightSamplingQuality::Rays32, "32"),
            (LightSamplingQuality::Rays64, "64"),
        ],
        "edit-scene-3d-light-sampling",
    )
}

fn vector(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<glam::Vec3>,
    runtime: InspectorRuntime,
    degrees: bool,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredVector3, path, label)
        .components(vec![
            value.x.to_string(),
            value.y.to_string(),
            value.z.to_string(),
        ])
        .number(NumberSpec {
            drag_step: if degrees { 1.0 } else { 0.1 },
            digits: 2,
            unit: if degrees { "°" } else { "" },
            ..NumberSpec::default()
        })
        .width_characters(5)
        .prefixes(["X", "Y", "Z"])
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::visual_modifiers::vector3_speed_graph(timeline, runtime),
        )
        .live_commit(VECTOR_COMMIT)
        .timeline_commits(VECTOR_COMMIT, VECTOR_EXPRESSION_COMMIT)
}

fn color_control(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<Color<u8>>,
    runtime: InspectorRuntime,
) -> InspectorControl {
    let value = timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO));
    InspectorControl::new(ControlKind::LayeredColor, path, label)
        .components(vec![
            value.r.to_string(),
            value.g.to_string(),
            value.b.to_string(),
            value.a.to_string(),
        ])
        .layered(path, LayeredState::from(timeline))
        .timeline(
            timeline.id,
            crate::timeline_color::speed_graph(timeline, runtime),
        )
        .live_commit(COLOR_COMMIT)
        .timeline_commits(COLOR_COMMIT, COLOR_COMMIT)
}

fn add_number(
    section: &mut InspectorSection,
    path: &'static str,
    label: &'static str,
    value: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    kind: ScalarKind,
) {
    section.add(scalar(path, label, value, runtime, kind));
}

fn scalar(
    path: &'static str,
    label: &'static str,
    timeline: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    kind: ScalarKind,
) -> InspectorControl {
    let stored = f64::from(timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO)));
    let mapping = if kind == ScalarKind::FocalLength {
        NumberMapping::FocalLengthMillimeters
    } else {
        NumberMapping::Linear
    };
    let number = scalar_spec(kind);
    let control = InspectorControl::new(ControlKind::LayeredNumber, path, label)
        .value(mapping.display(stored, 1.0).to_string())
        .number(number.clone())
        .number_constraint(scalar_constraint(kind, &number))
        .width_characters(9)
        .number_mapping(mapping)
        .layered(path, LayeredState::from(timeline))
        .timeline(timeline.id, scalar_graph(timeline, runtime, mapping))
        .live_commit(SCALAR_COMMIT)
        .timeline_commits(SCALAR_COMMIT, SCALAR_COMMIT);
    if kind.integer() {
        control.integer()
    } else {
        control
    }
}

fn scalar_graph(
    timeline: &TimelineValue<f32>,
    runtime: InspectorRuntime,
    mapping: NumberMapping,
) -> Option<crate::ScalarGraph> {
    let mut graph = crate::transform::scalar_graph(
        timeline,
        timeline.value_at(runtime.local_time.unwrap_or(Time::ZERO)),
        runtime,
    )?;
    for point in &mut graph.points {
        point.value = mapping.display(point.value, 1.0);
    }
    for segment in &mut graph.segments {
        segment.start_value = mapping.display(segment.start_value, 1.0);
        segment.end_value = mapping.display(segment.end_value, 1.0);
    }
    Some(graph)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScalarKind {
    Plain,
    Nonnegative,
    Positive,
    Unit,
    Bands,
    ColorLevels,
    KuwaharaRadius,
    ShadowTone,
    ShadowPatternSize,
    ShadowSoftness,
    DirectionCount,
    FocalLength,
    Fov,
    FisheyeFov,
    FStop,
    Exposure,
    OutlineWidth,
    PixelVariation,
    Frequency,
    Aggressiveness,
    Seed,
    RadiusRatio,
    DogSensitivity,
    DogSharpness,
    ClampedDegrees,
}

impl ScalarKind {
    fn integer(self) -> bool {
        matches!(
            self,
            Self::Bands
                | Self::ColorLevels
                | Self::KuwaharaRadius
                | Self::DirectionCount
                | Self::Seed
        )
    }
}

fn scalar_spec(kind: ScalarKind) -> NumberSpec {
    NumberSpec {
        drag_step: match kind {
            ScalarKind::Frequency | ScalarKind::DogSensitivity => 0.001,
            ScalarKind::Unit | ScalarKind::RadiusRatio => 0.01,
            kind if kind.integer() => 1.0,
            _ => 0.1,
        },
        digits: if matches!(kind, ScalarKind::Frequency | ScalarKind::DogSensitivity) {
            3
        } else if kind.integer() {
            0
        } else {
            2
        },
        minimum: match kind {
            ScalarKind::Nonnegative | ScalarKind::Unit => 0.0,
            ScalarKind::Positive => 0.001,
            ScalarKind::Bands | ScalarKind::ColorLevels => 2.0,
            ScalarKind::KuwaharaRadius => 0.0,
            ScalarKind::ShadowTone => f64::from(shrimply_scene_3d::MIN_TOON_SHADOW_TONE),
            ScalarKind::ShadowPatternSize => 0.25,
            ScalarKind::ShadowSoftness => 0.0,
            ScalarKind::DirectionCount => 1.0,
            ScalarKind::FocalLength | ScalarKind::Fov | ScalarKind::FisheyeFov => 1.0,
            ScalarKind::FStop => f64::from(shrimply_scene_3d::MIN_F_STOP),
            ScalarKind::Exposure => f64::from(shrimply_scene_3d::MIN_EXPOSURE_EV),
            ScalarKind::OutlineWidth => 0.25,
            ScalarKind::PixelVariation | ScalarKind::Seed => 0.0,
            ScalarKind::Frequency => 0.001,
            ScalarKind::Aggressiveness => 0.1,
            ScalarKind::RadiusRatio => 1.01,
            ScalarKind::DogSensitivity => 0.0,
            ScalarKind::DogSharpness => 1.0,
            ScalarKind::ClampedDegrees => 0.0,
            ScalarKind::Plain => NumberSpec::default().minimum,
        },
        maximum: match kind {
            ScalarKind::Unit => 1.0,
            ScalarKind::Bands => 16.0,
            ScalarKind::ColorLevels => 32.0,
            ScalarKind::KuwaharaRadius => 4.0,
            ScalarKind::ShadowTone => 1.0,
            ScalarKind::ShadowPatternSize => 64.0,
            ScalarKind::ShadowSoftness => 4.0,
            ScalarKind::DirectionCount => 6.0,
            ScalarKind::FocalLength => shrimply_scene_3d::focal_length_mm(1.0),
            ScalarKind::Fov => 179.0,
            ScalarKind::FisheyeFov => 360.0,
            ScalarKind::FStop => f64::from(shrimply_scene_3d::MAX_F_STOP),
            ScalarKind::Exposure => f64::from(shrimply_scene_3d::MAX_EXPOSURE_EV),
            ScalarKind::OutlineWidth | ScalarKind::PixelVariation => 16.0,
            ScalarKind::Frequency => 1.0,
            ScalarKind::Aggressiveness => 8.0,
            ScalarKind::Seed => f64::from(u32::MAX),
            ScalarKind::RadiusRatio => 4.0,
            ScalarKind::DogSensitivity => 0.25,
            ScalarKind::DogSharpness => 64.0,
            ScalarKind::ClampedDegrees => 180.0,
            _ => NumberSpec::default().maximum,
        },
        unit: match kind {
            ScalarKind::FocalLength => "mm",
            ScalarKind::Fov | ScalarKind::FisheyeFov | ScalarKind::ClampedDegrees => "deg",
            ScalarKind::OutlineWidth
            | ScalarKind::PixelVariation
            | ScalarKind::KuwaharaRadius
            | ScalarKind::ShadowPatternSize
            | ScalarKind::ShadowSoftness => "px",
            ScalarKind::Frequency => "/px",
            _ => "",
        },
    }
}

fn scalar_constraint(kind: ScalarKind, number: &NumberSpec) -> crate::NumberConstraint {
    let defaults = NumberSpec::default();
    crate::NumberConstraint {
        minimum: (number.minimum != defaults.minimum).then_some(number.minimum),
        maximum: if kind == ScalarKind::FocalLength {
            Some(179.0)
        } else {
            (number.maximum != defaults.maximum).then_some(number.maximum)
        },
        integer: kind.integer(),
    }
}

pub fn number(scene: &ObjScene, id: uuid::Uuid) -> Option<&TimelineValue<f32>> {
    camera_numbers(&scene.camera)
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers(&scene.material))
        .chain([&scene.environment.intensity])
        .find(|value| value.id == id)
}

pub fn number_mut(scene: &mut ObjScene, id: uuid::Uuid) -> Option<&mut TimelineValue<f32>> {
    camera_numbers_mut(&mut scene.camera)
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers_mut(&mut scene.material))
        .chain([&mut scene.environment.intensity])
        .find(|value| value.id == id)
}

fn camera_numbers(camera: &Camera3d) -> Vec<&TimelineValue<f32>> {
    vec![
        &camera.vertical_fov_degrees,
        &camera.orthographic_height,
        &camera.focus_distance,
        &camera.background_distance,
        &camera.background_intensity,
        &camera.f_stop,
        &camera.exposure_ev,
    ]
}
fn camera_numbers_mut(camera: &mut Camera3d) -> Vec<&mut TimelineValue<f32>> {
    vec![
        &mut camera.vertical_fov_degrees,
        &mut camera.orthographic_height,
        &mut camera.focus_distance,
        &mut camera.background_distance,
        &mut camera.background_intensity,
        &mut camera.f_stop,
        &mut camera.exposure_ev,
    ]
}

pub fn vector3(scene: &ObjScene, id: uuid::Uuid) -> Option<&TimelineValue<glam::Vec3>> {
    [
        &scene.camera.position,
        &scene.camera.rotation_degrees,
        &scene.environment.rotation_degrees,
    ]
    .into_iter()
    .find(|value| value.id == id)
}
pub fn vector3_mut(scene: &mut ObjScene, id: uuid::Uuid) -> Option<&mut TimelineValue<glam::Vec3>> {
    [
        &mut scene.camera.position,
        &mut scene.camera.rotation_degrees,
        &mut scene.environment.rotation_degrees,
    ]
    .into_iter()
    .find(|value| value.id == id)
}
pub fn color(scene: &ObjScene, id: uuid::Uuid) -> Option<&TimelineValue<Color<u8>>> {
    shrimply_scene_3d::material_colors(&scene.material)
        .into_iter()
        .chain([&scene.environment.solid_color])
        .find(|value| value.id == id)
}
pub fn color_mut(scene: &mut ObjScene, id: uuid::Uuid) -> Option<&mut TimelineValue<Color<u8>>> {
    shrimply_scene_3d::material_colors_mut(&mut scene.material)
        .into_iter()
        .chain([&mut scene.environment.solid_color])
        .find(|value| value.id == id)
}

pub fn is_timeline_path(path: &str) -> bool {
    path.starts_with("/content/material/toon/")
        || matches!(
            path,
            CAMERA_POSITION_PATH
                | CAMERA_ROTATION_PATH
                | CAMERA_FOV_PATH
                | CAMERA_FOCUS_DISTANCE_PATH
                | CAMERA_F_STOP_PATH
                | CAMERA_ORTHOGRAPHIC_HEIGHT_PATH
                | CAMERA_BACKGROUND_DISTANCE_PATH
                | CAMERA_BACKGROUND_INTENSITY_PATH
                | CAMERA_EXPOSURE_PATH
                | ENVIRONMENT_ROTATION_PATH
                | ENVIRONMENT_COLOR_PATH
                | ENVIRONMENT_INTENSITY_PATH
        )
}

impl InspectorController {
    pub fn set_scene_3d_environment(
        &self,
        target: &InspectorTarget,
        path: &std::path::Path,
    ) -> Result<(), String> {
        validate_environment(path)?;
        let mut project = self.project.borrow_mut();
        let scene = scene_mut(&mut project, target)?;
        if scene.environment.file.as_deref() == Some(path)
            && scene.environment.source == Some(EnvironmentSource::Image)
        {
            return Ok(());
        }
        scene.environment.file = Some(Asset::from(path));
        scene.environment.source = Some(EnvironmentSource::Image);
        shrimply_project_document::project::commit_edit(&project, "edit-scene-3d-environment-file");
        drop(project);
        refresh_scene(&self.player_state);
        Ok(())
    }

    pub fn clear_scene_3d_environment(&self, target: &InspectorTarget) -> Result<(), String> {
        let mut project = self.project.borrow_mut();
        let scene = scene_mut(&mut project, target)?;
        if scene.environment.file.is_none()
            && scene.environment.source == Some(EnvironmentSource::Composite)
        {
            return Ok(());
        }
        scene.environment.file = None;
        scene.environment.source = Some(EnvironmentSource::Composite);
        shrimply_project_document::project::commit_edit(
            &project,
            "clear-scene-3d-environment-file",
        );
        drop(project);
        refresh_scene(&self.player_state);
        Ok(())
    }
}

fn scene_mut<'a>(
    project: &'a mut Project,
    target: &InspectorTarget,
) -> Result<&'a mut ObjScene, String> {
    let InspectorTarget::Item(address @ ItemAddress::Video { .. }) = target else {
        return Err("scene 3D target is not a video item".to_string());
    };
    let item = project
        .video_item_mut(address)
        .ok_or_else(|| "video item is no longer available".to_string())?;
    let shrimply_project_document::project::VideoItemContent::Obj(scene) = &mut item.content else {
        return Err("scene 3D item is no longer available".to_string());
    };
    Ok(scene)
}

fn refresh_scene(player: &shrimply_editor_state::player_state::SharedPlayerState) {
    shrimply_editor_state::player_state::refresh_project(
        player,
        shrimply_editor_state::player_state::ProjectChange {
            video: true,
            inspector: true,
            ..Default::default()
        },
    );
}

fn validate_environment(path: &std::path::Path) -> Result<(), String> {
    let mut input = ffmpeg::format::input(path).map_err(|error| error.to_string())?;
    let stream = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| "image has no decodable video stream".to_string())?;
    let stream_index = stream.index();
    let mut decoder = ffmpeg::codec::context::Context::from_parameters(stream.parameters())
        .and_then(|context| context.decoder().video())
        .map_err(|error| error.to_string())?;
    let mut frame = ffmpeg::frame::Video::empty();
    for (stream, packet) in input.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder
            .send_packet(&packet)
            .map_err(|error| error.to_string())?;
        if decoder.receive_frame(&mut frame).is_ok() {
            return Ok(());
        }
    }
    decoder.send_eof().map_err(|error| error.to_string())?;
    decoder
        .receive_frame(&mut frame)
        .map_err(|error| format!("could not decode an image frame: {error}"))
}
