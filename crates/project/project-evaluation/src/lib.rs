mod expressions;
mod keyframes;

pub use shrimply_project_document::project;

use glam::Vec2;
use glam::Vec3;
use uuid::Uuid;

pub use expressions::{ExpressionDiagnostic, TransformExpressionCache, expression_symbols};
pub use keyframes::{color_keyframes_value, scalar_keyframes_value, vec2_keyframes_value};
pub use shrimply_lip_sync::{FrameAudioAnalysis, FrameMouthMixer, MouthShape, MouthValue};
use shrimply_math_core::Fraction;
pub use shrimply_math_media::FrameVolumeMixer;

use shrimply_project_document::project::{
    AudioItem, CanvasSize, PaintDrawing, PaintFillOptions, PaintStrokeEndOptions,
    PaintStrokeOptions, PaintTextureOptions, Project, ResolvedPaintFillOptions,
    ResolvedPaintStrokeEndOptions, ResolvedPaintStrokeOptions, ResolvedPaintTextureOptions,
    ResolvedTransform, Time, Transform, VideoItem, generated_item_animation_time,
    generated_item_keyframe_span, generated_item_time,
};
use shrimply_property_model::timeline_value::*;

#[derive(Clone)]
pub struct VisualEvaluation {
    pub(crate) item_id: Uuid,
    pub(crate) local_time: Time,
    pub(crate) expression_time: Time,
    pub(crate) duration: Time,
    pub(crate) item_start: Time,
    pub(crate) item_end: Time,
    pub(crate) fps: Fraction,
    pub(crate) canvas_size: CanvasSize,
    pub(crate) source_width: u32,
    pub(crate) source_height: u32,
    pub(crate) volume_mixer: FrameVolumeMixer,
    pub(crate) mouth_mixer: FrameMouthMixer,
    pub(crate) seed: u64,
    pub(crate) item_seed: u64,
}

impl VisualEvaluation {
    pub fn for_item(project: &Project, item: &VideoItem, position: Time) -> Self {
        let audio = FrameAudioAnalysis::silent(project.audio_tracks.len());
        Self::for_item_with_audio(project, item, position, &audio)
    }

    pub fn for_item_with_volume_mixer(
        project: &Project,
        item: &VideoItem,
        position: Time,
        volume_mixer: &FrameVolumeMixer,
    ) -> Self {
        let audio = FrameAudioAnalysis {
            volume: volume_mixer.clone(),
            mouth: FrameMouthMixer::silent(project.audio_tracks.len()),
        };
        Self::for_item_with_audio(project, item, position, &audio)
    }

    pub fn for_item_with_audio(
        project: &Project,
        item: &VideoItem,
        position: Time,
        audio: &FrameAudioAnalysis,
    ) -> Self {
        let local_time = generated_item_time(item, position).unwrap_or(Time::ZERO);
        Self::for_item_at_local_time_with_audio(project, item, position, local_time, audio)
    }

    pub fn for_item_local_time(project: &Project, item: &VideoItem, local_time: Time) -> Self {
        let audio = FrameAudioAnalysis::silent(project.audio_tracks.len());
        Self::for_item_local_time_with_audio(project, item, local_time, &audio)
    }

    pub fn for_audio_item_local_time(
        project: &Project,
        item: &AudioItem,
        local_time: Time,
    ) -> Self {
        let expression_nanos = local_time.as_nanos_i128() as u64;
        Self {
            item_id: item.id,
            local_time,
            expression_time: local_time,
            duration: item
                .end
                .saturating_sub(item.start)
                .max(item.start.saturating_sub(item.end)),
            item_start: item.start,
            item_end: item.end,
            fps: project.fps,
            canvas_size: project.canvas_size,
            source_width: 0,
            source_height: 0,
            volume_mixer: FrameVolumeMixer::silent(project.audio_tracks.len()),
            mouth_mixer: FrameMouthMixer::silent(project.audio_tracks.len()),
            seed: expression_nanos ^ item.id.as_u128() as u64,
            item_seed: item.id.as_u128() as u64,
        }
    }

    pub fn for_item_local_time_with_volume_mixer(
        project: &Project,
        item: &VideoItem,
        local_time: Time,
        volume_mixer: &FrameVolumeMixer,
    ) -> Self {
        let audio = FrameAudioAnalysis {
            volume: volume_mixer.clone(),
            mouth: FrameMouthMixer::silent(project.audio_tracks.len()),
        };
        Self::for_item_local_time_with_audio(project, item, local_time, &audio)
    }

    pub fn for_item_local_time_with_audio(
        project: &Project,
        item: &VideoItem,
        local_time: Time,
        audio: &FrameAudioAnalysis,
    ) -> Self {
        Self::with_times(project, item, local_time, local_time, audio)
    }

    pub fn for_item_at_local_time_with_audio(
        project: &Project,
        item: &VideoItem,
        position: Time,
        local_time: Time,
        audio: &FrameAudioAnalysis,
    ) -> Self {
        Self::with_times(
            project,
            item,
            local_time,
            generated_item_animation_time(item, position),
            audio,
        )
    }

    fn with_times(
        project: &Project,
        item: &VideoItem,
        local_time: Time,
        expression_time: Time,
        audio: &FrameAudioAnalysis,
    ) -> Self {
        let expression_nanos = expression_time.as_nanos_i128() as u64;
        let duration = generated_item_keyframe_span(item)
            .map(|(start, end)| end.saturating_sub(start))
            .unwrap_or_else(|| item.end.saturating_sub(item.start));
        Self {
            item_id: item.id,
            local_time,
            expression_time,
            duration,
            item_start: item.start,
            item_end: item.end,
            fps: if item.playback_fps == shrimply_project_document::project::native_playback_fps() {
                project.fps
            } else {
                item.playback_fps
            },
            canvas_size: project.canvas_size,
            source_width: item.source_width,
            source_height: item.source_height,
            volume_mixer: audio.volume.clone(),
            mouth_mixer: audio.mouth.clone(),
            seed: expression_nanos ^ item.id.as_u128() as u64,
            item_seed: item.id.as_u128() as u64,
        }
    }

    pub fn local_time(&self) -> Time {
        self.local_time
    }

    pub fn at_local_time(&self, local_time: Time) -> Self {
        let mut evaluation = self.clone();
        evaluation.local_time = local_time;
        evaluation.expression_time = local_time;
        evaluation.seed = local_time.as_nanos_i128() as u64 ^ evaluation.item_id.as_u128() as u64;
        evaluation
    }
}

pub type TransformEvaluation = VisualEvaluation;

pub fn resolve<T: TimelineExpressionValue>(
    value: &TimelineValue<T>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> T {
    resolve_with_error(value, eval, cache).value
}

pub struct ExpressionOutcome<T> {
    pub value: T,
    pub error: Option<String>,
}

pub fn resolve_with_error<T: TimelineExpressionValue>(
    value: &TimelineValue<T>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> ExpressionOutcome<T> {
    let base = resolve_base(value, eval);
    let Some(expression) = &value.expression else {
        return ExpressionOutcome {
            value: base,
            error: None,
        };
    };
    if !expression.enabled || expression.source.trim().is_empty() {
        return ExpressionOutcome {
            value: base,
            error: None,
        };
    }
    match cache.eval_timeline_value_result(eval, value.id, &expression.source, &base) {
        Ok(value) => ExpressionOutcome { value, error: None },
        Err(error) => ExpressionOutcome {
            value: base,
            error: Some(error),
        },
    }
}

pub fn resolve_base<T: TimelineExpressionValue>(
    value: &TimelineValue<T>,
    eval: &TransformEvaluation,
) -> T {
    value.value_at(eval.local_time)
}

pub fn resolve_paint_drawing(
    value: &TimelineValue<PaintDrawing>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
    palette_len: usize,
) -> PaintDrawing {
    let base = value.value_at(eval.local_time);
    let drawing = resolve(value, eval, cache);
    let valid = palette_len > 0
        && drawing
            .strokes
            .iter()
            .all(|stroke| stroke.color_index < palette_len)
        && drawing
            .fills
            .iter()
            .all(|fill| fill.color_index < palette_len);
    if valid { drawing } else { base }
}

/// Evaluates an expression value without exposing the expression engine to callers.
pub fn evaluate_expression<T: TimelineExpressionValue>(
    cache: &mut TransformExpressionCache,
    eval: &TransformEvaluation,
    value_id: Uuid,
    source: &str,
    base: &T,
) -> Option<T> {
    cache.eval_timeline_value(eval, value_id, source, base)
}

pub fn resolve_obj_scene(
    scene: &shrimply_scene_3d::ObjScene,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> shrimply_scene_3d::ResolvedObjScene {
    use shrimply_scene_3d::{
        ResolvedCamera3d, ResolvedEnvironment3d, ResolvedObjScene, ResolvedPbrMaterial,
        ResolvedShadowReceiverPlane3d, ResolvedToonMaterial, ResolvedToonOutline,
        ResolvedTransform3d,
    };

    ResolvedObjScene {
        model: ResolvedTransform3d {
            position: resolve_vec3(&scene.model.position, eval, cache),
            anchor: resolve_vec3(&scene.model.anchor, eval, cache),
            rotation_degrees: resolve_vec3(&scene.model.rotation_degrees, eval, cache),
            rotation_order: resolve(&scene.model.rotation_order, eval, cache),
            scale: resolve_vec3(&scene.model.scale, eval, cache),
        },
        camera: ResolvedCamera3d {
            projection: scene.camera.projection,
            anti_aliasing: scene.camera.anti_aliasing,
            position: resolve_vec3(&scene.camera.position, eval, cache),
            rotation_degrees: resolve_vec3(&scene.camera.rotation_degrees, eval, cache),
            vertical_fov_degrees: resolve_scalar(&scene.camera.vertical_fov_degrees, eval, cache),
            orthographic_height: resolve_scalar(&scene.camera.orthographic_height, eval, cache),
            focus_distance: resolve_scalar(&scene.camera.focus_distance, eval, cache),
            background_distance: resolve_scalar(&scene.camera.background_distance, eval, cache),
            background_plane_enabled: scene.camera.background_plane_enabled,
            background_intensity: resolve_scalar(&scene.camera.background_intensity, eval, cache),
            background_address_mode: scene.camera.background_address_mode,
            f_stop: resolve_scalar(&scene.camera.f_stop, eval, cache),
            exposure_ev: resolve_scalar(&scene.camera.exposure_ev, eval, cache),
        },
        material: ResolvedPbrMaterial {
            base_color: resolve_color(&scene.material.base_color, eval, cache),
            metallic: resolve_scalar(&scene.material.metallic, eval, cache),
            roughness: resolve_scalar(&scene.material.roughness, eval, cache),
            subsurface: resolve_scalar(&scene.material.subsurface, eval, cache),
            clearcoat: resolve_scalar(&scene.material.clearcoat, eval, cache),
            sheen: resolve_scalar(&scene.material.sheen, eval, cache),
            transmission: resolve_scalar(&scene.material.transmission, eval, cache),
            ior: resolve_scalar(&scene.material.ior, eval, cache),
            path_tracing: scene.material.path_tracing,
            light_sampling_quality: scene.material.light_sampling_quality,
            denoising: scene.material.denoising,
            normal_mode: scene.material.normal_mode,
            shading_model: scene.material.shading_model,
            toon: ResolvedToonMaterial {
                bands: resolve_scalar(&scene.material.toon.bands, eval, cache),
                texture_filter: scene.material.toon.texture_filter,
                color_levels: resolve_scalar(&scene.material.toon.color_levels, eval, cache),
                kuwahara_radius: resolve_scalar(&scene.material.toon.kuwahara_radius, eval, cache),
                kuwahara_strength: resolve_scalar(
                    &scene.material.toon.kuwahara_strength,
                    eval,
                    cache,
                ),
                shadow_kind: scene.material.toon.shadow_kind,
                shadow_color: resolve_color(&scene.material.toon.shadow_color, eval, cache),
                shadow_strength: resolve_scalar(&scene.material.toon.shadow_strength, eval, cache),
                shadow_darkest_tone: resolve_scalar(
                    &scene.material.toon.shadow_darkest_tone,
                    eval,
                    cache,
                ),
                shadow_dot_size: resolve_scalar(&scene.material.toon.shadow_dot_size, eval, cache),
                shadow_dot_density: resolve_scalar(
                    &scene.material.toon.shadow_dot_density,
                    eval,
                    cache,
                ),
                shadow_dot_distribution_randomness: resolve_scalar(
                    &scene.material.toon.shadow_dot_distribution_randomness,
                    eval,
                    cache,
                ),
                shadow_dot_size_randomness: resolve_scalar(
                    &scene.material.toon.shadow_dot_size_randomness,
                    eval,
                    cache,
                ),
                shadow_line_direction_degrees: resolve_scalar(
                    &scene.material.toon.shadow_line_direction_degrees,
                    eval,
                    cache,
                ),
                shadow_line_width: resolve_scalar(
                    &scene.material.toon.shadow_line_width,
                    eval,
                    cache,
                ),
                shadow_line_density: resolve_scalar(
                    &scene.material.toon.shadow_line_density,
                    eval,
                    cache,
                ),
                shadow_line_distribution_randomness: resolve_scalar(
                    &scene.material.toon.shadow_line_distribution_randomness,
                    eval,
                    cache,
                ),
                shadow_line_width_randomness: resolve_scalar(
                    &scene.material.toon.shadow_line_width_randomness,
                    eval,
                    cache,
                ),
                shadow_pattern_softness: resolve_scalar(
                    &scene.material.toon.shadow_pattern_softness,
                    eval,
                    cache,
                ),
                shadow_crosshatch_angle_degrees: resolve_scalar(
                    &scene.material.toon.shadow_crosshatch_angle_degrees,
                    eval,
                    cache,
                ),
                shadow_crosshatch_max_directions: resolve_scalar(
                    &scene.material.toon.shadow_crosshatch_max_directions,
                    eval,
                    cache,
                ),
                rim_color: resolve_color(&scene.material.toon.rim_color, eval, cache),
                rim_strength: resolve_scalar(&scene.material.toon.rim_strength, eval, cache),
                rim_power: resolve_scalar(&scene.material.toon.rim_power, eval, cache),
                specular_size: resolve_scalar(&scene.material.toon.specular_size, eval, cache),
                specular_strength: resolve_scalar(
                    &scene.material.toon.specular_strength,
                    eval,
                    cache,
                ),
                outline: ResolvedToonOutline {
                    mode: scene.material.toon.outline.mode,
                    method: scene.material.toon.outline.method,
                    quality: scene.material.toon.outline.quality,
                    color: resolve_color(&scene.material.toon.outline.color, eval, cache),
                    width: resolve_scalar(&scene.material.toon.outline.width, eval, cache),
                    opacity: resolve_scalar(&scene.material.toon.outline.opacity, eval, cache),
                    depth_threshold: resolve_scalar(
                        &scene.material.toon.outline.depth_threshold,
                        eval,
                        cache,
                    ),
                    normal_angle_degrees: resolve_scalar(
                        &scene.material.toon.outline.normal_angle_degrees,
                        eval,
                        cache,
                    ),
                    dog_inner_radius: resolve_scalar(
                        &scene.material.toon.outline.dog_inner_radius,
                        eval,
                        cache,
                    ),
                    dog_radius_ratio: resolve_scalar(
                        &scene.material.toon.outline.dog_radius_ratio,
                        eval,
                        cache,
                    ),
                    dog_threshold: resolve_scalar(
                        &scene.material.toon.outline.dog_threshold,
                        eval,
                        cache,
                    ),
                    dog_sharpness: resolve_scalar(
                        &scene.material.toon.outline.dog_sharpness,
                        eval,
                        cache,
                    ),
                    offset_variation: resolve_scalar(
                        &scene.material.toon.outline.offset_variation,
                        eval,
                        cache,
                    ),
                    width_variation: resolve_scalar(
                        &scene.material.toon.outline.width_variation,
                        eval,
                        cache,
                    ),
                    offset_frequency: resolve_scalar(
                        &scene.material.toon.outline.offset_frequency,
                        eval,
                        cache,
                    ),
                    width_frequency: resolve_scalar(
                        &scene.material.toon.outline.width_frequency,
                        eval,
                        cache,
                    ),
                    aggressiveness: resolve_scalar(
                        &scene.material.toon.outline.aggressiveness,
                        eval,
                        cache,
                    ),
                    noise_seed: resolve_scalar(
                        &scene.material.toon.outline.noise_seed,
                        eval,
                        cache,
                    ),
                    noise_evolution: resolve_scalar(
                        &scene.material.toon.outline.noise_evolution,
                        eval,
                        cache,
                    ),
                },
            },
        },
        shadow_receiver: ResolvedShadowReceiverPlane3d {
            enabled: resolve_bool(&scene.shadow_receiver.enabled, eval, cache),
            composite_enabled: scene.shadow_receiver.composite_enabled,
            intensity: resolve_scalar(&scene.shadow_receiver.intensity, eval, cache),
            position: resolve_vec3(&scene.shadow_receiver.position, eval, cache),
            rotation_degrees: resolve_vec3(&scene.shadow_receiver.rotation_degrees, eval, cache),
            opacity: resolve_scalar(&scene.shadow_receiver.opacity, eval, cache),
            shadow_strength: resolve_scalar(&scene.shadow_receiver.shadow_strength, eval, cache),
            reflection: resolve_scalar(&scene.shadow_receiver.reflection, eval, cache),
            roughness: resolve_scalar(&scene.shadow_receiver.roughness, eval, cache),
        },
        environment: ResolvedEnvironment3d {
            source: scene.environment.effective_source(),
            file: scene.environment.file.clone(),
            solid_color: resolve_color(&scene.environment.solid_color, eval, cache),
            rotation_degrees: resolve_vec3(&scene.environment.rotation_degrees, eval, cache),
            intensity: resolve_scalar(&scene.environment.intensity, eval, cache),
        },
    }
}

pub fn resolve_gaussian_scene(
    scene: &shrimply_3dgs_core::GaussianScene,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> shrimply_3dgs_core::RenderParams {
    let rotation_order = resolve(&scene.model.rotation_order, eval, cache);
    shrimply_3dgs_core::RenderParams {
        model: shrimply_3dgs_core::Transform {
            position: resolve_vec3(&scene.model.position, eval, cache),
            anchor: resolve_vec3(&scene.model.anchor, eval, cache),
            rotation_degrees: resolve_vec3(&scene.model.rotation_degrees, eval, cache),
            rotation_order,
            scale: resolve_vec3(&scene.model.scale, eval, cache),
        },
        camera: shrimply_3dgs_core::Camera {
            projection: scene.camera.projection,
            position: resolve_vec3(&scene.camera.position, eval, cache),
            rotation_degrees: resolve_vec3(&scene.camera.rotation_degrees, eval, cache),
            vertical_fov_degrees: resolve_scalar(&scene.camera.vertical_fov_degrees, eval, cache),
            orthographic_height: resolve_scalar(&scene.camera.orthographic_height, eval, cache),
            focus_distance: resolve_scalar(&scene.camera.focus_distance, eval, cache),
            f_stop: resolve_scalar(&scene.camera.f_stop, eval, cache),
            exposure_ev: resolve_scalar(&scene.camera.exposure_ev, eval, cache),
        },
    }
}

fn resolve_vec3(
    value: &TimelineValue<Vec3>,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> Vec3 {
    resolve(value, eval, cache)
}

pub fn resolve_item_transform(
    project: &Project,
    item: &VideoItem,
    position: Time,
    cache: &mut TransformExpressionCache,
) -> ResolvedTransform {
    let volume_mixer = FrameVolumeMixer::silent(project.audio_tracks.len());
    resolve_item_transform_with_volume_mixer(project, item, position, &volume_mixer, cache)
}

pub fn resolve_item_transform_with_volume_mixer(
    project: &Project,
    item: &VideoItem,
    position: Time,
    volume_mixer: &FrameVolumeMixer,
    cache: &mut TransformExpressionCache,
) -> ResolvedTransform {
    let audio = FrameAudioAnalysis {
        volume: volume_mixer.clone(),
        mouth: FrameMouthMixer::silent(project.audio_tracks.len()),
    };
    resolve_item_transform_with_audio(project, item, position, &audio, cache)
}

pub fn resolve_item_transform_with_audio(
    project: &Project,
    item: &VideoItem,
    position: Time,
    audio: &FrameAudioAnalysis,
    cache: &mut TransformExpressionCache,
) -> ResolvedTransform {
    let eval = TransformEvaluation::for_item_with_audio(project, item, position, audio);
    resolve_transform(&item.transform, &eval, cache)
}

pub fn resolve_item_base_transform(
    project: &Project,
    item: &VideoItem,
    position: Time,
) -> ResolvedTransform {
    let eval = TransformEvaluation::for_item(project, item, position);
    resolve_base_transform(&item.transform, &eval)
}

pub fn resolve_transform(
    transform: &Transform,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> ResolvedTransform {
    ResolvedTransform {
        position: resolve_vec2(&transform.position, eval, cache),
        anchor: resolve_vec2(&transform.anchor, eval, cache),
        scale: resolve_vec2(&transform.scale, eval, cache),
        shear: resolve_vec2(&transform.shear, eval, cache),
        rotation_degrees: resolve_scalar(&transform.rotation_degrees, eval, cache),
    }
}

pub fn resolve_base_transform(
    transform: &Transform,
    eval: &TransformEvaluation,
) -> ResolvedTransform {
    ResolvedTransform {
        position: resolve_base(&transform.position, eval),
        anchor: resolve_base(&transform.anchor, eval),
        scale: resolve_base(&transform.scale, eval),
        shear: resolve_base(&transform.shear, eval),
        rotation_degrees: resolve_base(&transform.rotation_degrees, eval),
    }
}

pub fn resolve_scalar(
    value: &TimelineValue<f32>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> f32 {
    resolve(value, eval, cache)
}

pub fn resolve_vec2(
    value: &TimelineValue<glam::Vec2>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> Vec2 {
    resolve(value, eval, cache)
}

pub fn resolve_color(
    value: &TimelineValue<shrimply_property_model::Color<u8>>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> shrimply_project_document::project::Color<u8> {
    resolve(value, eval, cache)
}

pub fn resolve_text(
    value: &TimelineValue<String>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> String {
    resolve(value, eval, cache)
}

pub fn resolve_bool(
    value: &TimelineValue<TimelineBool>,
    eval: &TransformEvaluation,
    cache: &mut TransformExpressionCache,
) -> bool {
    resolve(value, eval, cache).get()
}

pub fn resolve_paint_stroke_options(
    stroke: &PaintStrokeOptions,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> ResolvedPaintStrokeOptions {
    ResolvedPaintStrokeOptions {
        width: resolve_scalar(&stroke.width, eval, cache),
        thinning: resolve_scalar(&stroke.thinning, eval, cache),
        smoothing: resolve_scalar(&stroke.smoothing, eval, cache),
        streamline: resolve_scalar(&stroke.streamline, eval, cache),
        simplification_tolerance: resolve_scalar(&stroke.simplification_tolerance, eval, cache),
        maximum_subdivision_spacing: resolve_scalar(
            &stroke.maximum_subdivision_spacing,
            eval,
            cache,
        ),
        start: resolve_paint_stroke_end(&stroke.start, eval, cache),
        end: resolve_paint_stroke_end(&stroke.end, eval, cache),
    }
}

pub fn resolve_paint_fill_options(
    fill: &PaintFillOptions,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> ResolvedPaintFillOptions {
    ResolvedPaintFillOptions {
        closure_tolerance: resolve_scalar(&fill.closure_tolerance, eval, cache),
    }
}

pub fn resolve_paint_texture_options(
    texture: &PaintTextureOptions,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> ResolvedPaintTextureOptions {
    ResolvedPaintTextureOptions {
        repeat_scale: resolve_scalar(&texture.repeat_scale, eval, cache),
        rotation_degrees: resolve_scalar(&texture.rotation_degrees, eval, cache),
    }
}

pub fn resolve_path_offset_modifier(
    modifier: &shrimply_visual_modifiers::path_offset::PathOffsetModifier,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> shrimply_paint_geometry::ResolvedPathOffset {
    shrimply_paint_geometry::ResolvedPathOffset {
        amplitude: resolve_scalar(&modifier.amplitude, eval, cache).max(0.0),
        spacing: resolve_scalar(&modifier.spacing, eval, cache).max(0.1),
        seed: resolve_scalar(&modifier.seed, eval, cache),
        evolution: resolve_scalar(&modifier.evolution, eval, cache),
    }
}

fn resolve_paint_stroke_end(
    end: &PaintStrokeEndOptions,
    eval: &VisualEvaluation,
    cache: &mut TransformExpressionCache,
) -> ResolvedPaintStrokeEndOptions {
    ResolvedPaintStrokeEndOptions {
        cap: resolve_bool(&end.cap, eval, cache),
        taper: resolve(&end.taper, eval, cache),
        taper_distance: resolve_scalar(&end.taper_distance, eval, cache),
    }
}
