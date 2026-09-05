use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    rc::Rc,
};

use hashbrown::HashMap;
use shrimply_asset::{Asset, AssetSnapshot};
use shrimply_project::project::{CanvasSize, VideoItem, VideoItemContent};
use shrimply_video_modifiers::{ModifierEffect, scene_3d::Scene3dModifierEffect};
use uuid::Uuid;

use crate::gpu::CudaVideoCompositor;
use crate::layer::{GpuFrame, RasterVisual, Visual};
use crate::visual_source::{VisualElement, VisualRender, VisualRenderRequest, VisualSourceCache};

pub struct ObjElement {
    sessions: HashMap<Asset, shrimply_render_3d::ObjRenderSession>,
    text_sessions: HashMap<Uuid, TextSession>,
    shape_sessions: HashMap<Uuid, ShapeSession>,
    composed: Option<shrimply_render_3d::ObjRenderSession>,
    expressions: shrimply_evaluation::TransformExpressionCache,
    cached: Option<CachedObjFrame>,
}

struct TextSession {
    key: u64,
    session: shrimply_render_3d::ObjRenderSession,
}

struct ShapeSession {
    key: u64,
    session: shrimply_render_3d::ObjRenderSession,
}

enum ObjectSource {
    File(Asset),
    Text(Uuid),
    Shape(Uuid),
}

struct CachedObjFrame {
    renderer_generation: u64,
    width: u32,
    height: u32,
    environment: Option<AssetSnapshot>,
    scene: shrimply_render_3d::SceneIdentity,
    uniforms: shrimply_render_3d::obj::SceneUniforms,
    layer: Rc<crate::gpu::VisualFrame>,
}

impl ObjElement {
    pub fn new(_item: &VideoItem) -> Result<Self, String> {
        Ok(Self {
            sessions: HashMap::new(),
            text_sessions: HashMap::new(),
            shape_sessions: HashMap::new(),
            composed: None,
            expressions: Default::default(),
            cached: None,
        })
    }
}

impl VisualElement for ObjElement {
    fn matches(&self, item: &VideoItem, _canvas_size: CanvasSize) -> bool {
        matches!(&item.content, VideoItemContent::Obj(_))
    }

    fn draw(
        &mut self,
        request: VisualRenderRequest<'_>,
        compositor: &mut CudaVideoCompositor,
        track_id: Uuid,
        _cache: &mut VisualSourceCache,
    ) -> Result<VisualRender, String> {
        let VideoItemContent::Obj(scene) = &request.item.content else {
            return Err("OBJ renderer received a non-OBJ visual".to_string());
        };
        let camera_source = scene.camera.source.clone();
        let evaluation = shrimply_evaluation::VisualEvaluation::for_item_with_audio(
            request.project,
            request.item,
            request.position,
            request.audio_analysis,
        );
        let resolved_scene =
            shrimply_evaluation::resolve_obj_scene(scene, &evaluation, &mut self.expressions);
        let mut params = shrimply_render_3d::SceneRenderParams::from(&resolved_scene);
        params.grounds.clear();
        params.shadow_receiver_enabled = false;
        let mut objects = Vec::new();
        if let shrimply_scene_3d::CameraSource::Tracking(source) = &camera_source
            && source.track_id != track_id
            && request
                .project
                .video_tracks
                .iter()
                .any(|track| track.id == source.track_id)
            && let Some(camera) = crate::camera_reconstruction::sample(
                request.item.id,
                source,
                request
                    .position
                    .signed_sub(request.item.start)
                    .saturating_add(request.item.animation_time_offset),
            )
        {
            let camera = crate::camera_reconstruction::apply_custom_camera_offset(
                camera,
                params.camera_position,
                params.camera_rotation_degrees,
            );
            params.camera_position = camera.position;
            params.camera_rotation_degrees = shrimply_transform_3d::rotation_degrees(
                camera.rotation,
                shrimply_transform_3d::RotationOrder::Xyz,
            );
            params.camera_projection = camera.projection;
            params.vertical_fov_degrees = camera.vertical_fov_degrees;
        }
        for modifier in request
            .item
            .modifiers
            .iter()
            .filter(|modifier| modifier.enabled)
        {
            let ModifierEffect::Scene3d(effect) = &modifier.effect else {
                if matches!(&modifier.effect, ModifierEffect::Rasterize(_)) {
                    break;
                }
                continue;
            };
            match &**effect {
                Scene3dModifierEffect::Object(object) => {
                    let Some(path) = object.file.clone() else {
                        continue;
                    };
                    let object_scene = shrimply_scene_3d::ObjScene {
                        model: object.transform.clone(),
                        material: object.material.clone(),
                        ..(**scene).clone()
                    };
                    let resolved = shrimply_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::File(path), resolved));
                }
                Scene3dModifierEffect::Text(text) => {
                    let content = shrimply_evaluation::resolve_text(
                        &text.text,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let font_size = shrimply_evaluation::resolve(
                        &text.font_size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let font_weight = shrimply_evaluation::resolve(
                        &text.font_weight,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let depth = shrimply_evaluation::resolve(
                        &text.depth,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let roundness = shrimply_evaluation::resolve(
                        &text.roundness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let smoothness = shrimply_evaluation::resolve(
                        &text.smoothness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let geometry = shrimply_text_3d::Geometry {
                        text: &content,
                        font_families: &text.font_families,
                        font_style: text.font_style,
                        font_variations: &text.font_variations,
                        font_weight,
                        h_align: text.h_align,
                        v_align: text.v_align,
                        direction: text.direction,
                        font_size,
                        depth,
                        roundness,
                        smoothness,
                    };
                    let key = text_geometry_key(&geometry);
                    if self
                        .text_sessions
                        .get(&modifier.id)
                        .is_none_or(|session| session.key != key)
                    {
                        let mesh = shrimply_text_3d::generate_mesh(&geometry)
                            .map_err(|error| error.to_string())?;
                        self.text_sessions.insert(
                            modifier.id,
                            TextSession {
                                key,
                                session: shrimply_render_3d::ObjRenderSession::generated(
                                    "<3D text>",
                                    vec![key as u32, (key >> 32) as u32],
                                    mesh,
                                ),
                            },
                        );
                    }
                    let object_scene = shrimply_scene_3d::ObjScene {
                        model: text.transform.clone(),
                        material: text.material.clone(),
                        ..(**scene).clone()
                    };
                    let resolved = shrimply_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::Text(modifier.id), resolved));
                }
                Scene3dModifierEffect::Shape(shape) => {
                    let size = shrimply_evaluation::resolve(
                        &shape.size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let corner_radius = shrimply_evaluation::resolve(
                        &shape.corner_radius,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let edge_roundness = shrimply_evaluation::resolve(
                        &shape.edge_roundness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let smoothness = shrimply_evaluation::resolve(
                        &shape.smoothness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let star_points = shrimply_evaluation::resolve(
                        &shape.star_points,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let star_inner_radius_percent = shrimply_evaluation::resolve(
                        &shape.star_inner_radius_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let arrow_shaft_width_percent = shrimply_evaluation::resolve(
                        &shape.arrow_shaft_width_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let arrow_head_length_percent = shrimply_evaluation::resolve(
                        &shape.arrow_head_length_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let cross_arm_thickness_percent = shrimply_evaluation::resolve(
                        &shape.cross_arm_thickness_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let disk_inner_radius_percent = shrimply_evaluation::resolve(
                        &shape.disk_inner_radius_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let disk_completion_degrees = shrimply_evaluation::resolve(
                        &shape.disk_completion_degrees,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let torus_inner_radius_percent = shrimply_evaluation::resolve(
                        &shape.torus_inner_radius_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let geometry = shrimply_shape_3d::Geometry {
                        shape: shape.shape,
                        size,
                        corner_radius,
                        rounding_strategy: shape.rounding_strategy,
                        edge_roundness,
                        smoothness,
                        star_points,
                        star_inner_radius_percent,
                        arrow_shaft_width_percent,
                        arrow_head_length_percent,
                        cross_arm_thickness_percent,
                        disk_inner_radius_percent,
                        disk_completion_degrees,
                        torus_inner_radius_percent,
                    };
                    let key = shape_geometry_key(geometry);
                    if self
                        .shape_sessions
                        .get(&modifier.id)
                        .is_none_or(|session| session.key != key)
                    {
                        let mesh = shrimply_shape_3d::generate_mesh(geometry)
                            .map_err(|error| error.to_string())?;
                        self.shape_sessions.insert(
                            modifier.id,
                            ShapeSession {
                                key,
                                session: shrimply_render_3d::ObjRenderSession::generated(
                                    "<3D shape>",
                                    vec![key as u32, (key >> 32) as u32],
                                    mesh,
                                ),
                            },
                        );
                    }
                    let object_scene = shrimply_scene_3d::ObjScene {
                        model: shape.transform.clone(),
                        material: shape.material.clone(),
                        ..(**scene).clone()
                    };
                    let resolved = shrimply_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::Shape(modifier.id), resolved));
                }
                Scene3dModifierEffect::Ground(ground) => {
                    let intensity = shrimply_evaluation::resolve(
                        &ground.intensity,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let position = shrimply_evaluation::resolve(
                        &ground.position,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let rotation_degrees = shrimply_evaluation::resolve(
                        &ground.rotation_degrees,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let opacity = shrimply_evaluation::resolve(
                        &ground.opacity,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let shadow_strength = shrimply_evaluation::resolve(
                        &ground.shadow_strength,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let reflection = shrimply_evaluation::resolve(
                        &ground.reflection,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let roughness = shrimply_evaluation::resolve(
                        &ground.roughness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let size = shrimply_evaluation::resolve(
                        &ground.size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params.grounds.push(shrimply_render_3d::GroundParams {
                        shape: match ground.kind {
                            shrimply_video_modifiers::scene_3d::GroundKind::Infinite => {
                                shrimply_render_3d::GroundShape::Infinite
                            }
                            shrimply_video_modifiers::scene_3d::GroundKind::Square => {
                                shrimply_render_3d::GroundShape::Square
                            }
                        },
                        size,
                        composite_enabled: ground.composite_enabled,
                        intensity,
                        position,
                        rotation_degrees,
                        opacity,
                        shadow_strength,
                        reflection,
                        roughness,
                    });
                }
                Scene3dModifierEffect::PointLight(light) => {
                    let color = shrimply_evaluation::resolve(
                        &light.color,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params
                        .point_lights
                        .push(shrimply_render_3d::PointLightParams {
                            position: shrimply_evaluation::resolve(
                                &light.position,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            color_linear: color.to_linear(),
                            intensity: shrimply_evaluation::resolve(
                                &light.intensity,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            range: shrimply_evaluation::resolve(
                                &light.range,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            radius: shrimply_evaluation::resolve(
                                &light.radius,
                                &evaluation,
                                &mut self.expressions,
                            ),
                        });
                }
                Scene3dModifierEffect::SunLight(light) => {
                    let color = shrimply_evaluation::resolve(
                        &light.color,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params.sun_lights.push(shrimply_render_3d::SunLightParams {
                        rotation_degrees: shrimply_evaluation::resolve(
                            &light.rotation_degrees,
                            &evaluation,
                            &mut self.expressions,
                        ),
                        color_linear: color.to_linear(),
                        intensity: shrimply_evaluation::resolve(
                            &light.intensity,
                            &evaluation,
                            &mut self.expressions,
                        ),
                        angular_radius_degrees: shrimply_evaluation::resolve(
                            &light.angular_radius_degrees,
                            &evaluation,
                            &mut self.expressions,
                        ),
                    });
                }
            }
        }
        let paths = objects
            .iter()
            .filter_map(|(source, _)| match source {
                ObjectSource::File(path) => Some(path.clone()),
                ObjectSource::Text(_) | ObjectSource::Shape(_) => None,
            })
            .collect::<Vec<_>>();
        for path in paths {
            let current = self
                .sessions
                .get(&path)
                .map(|session| session.matches_asset(&path))
                .transpose()
                .map_err(|error| error.to_string())?
                .unwrap_or(false);
            if !current {
                self.sessions.insert(
                    path.clone(),
                    shrimply_render_3d::ObjRenderSession::load(&path)
                        .map_err(|error| error.to_string())?,
                );
            }
        }
        let scene_objects = objects
            .iter()
            .map(|(source, scene)| shrimply_render_3d::SceneObject {
                session: match source {
                    ObjectSource::File(path) => self
                        .sessions
                        .get(path)
                        .expect("configured 3D object session is loaded"),
                    ObjectSource::Text(id) => {
                        &self
                            .text_sessions
                            .get(id)
                            .expect("configured 3D text session is generated")
                            .session
                    }
                    ObjectSource::Shape(id) => {
                        &self
                            .shape_sessions
                            .get(id)
                            .expect("configured 3D shape session is generated")
                            .session
                    }
                },
                transform: scene.model,
                material: shrimply_render_3d::SurfaceMaterialParams::from(scene),
            })
            .collect::<Vec<_>>();
        if let Some(ground) = params.grounds.first() {
            params.shadow_receiver_enabled = true;
            params.ground_composite_enabled = ground.composite_enabled;
            params.ground_intensity = ground.intensity;
            params.shadow_receiver_position = ground.position;
            params.shadow_receiver_rotation_degrees = ground.rotation_degrees;
            params.shadow_receiver_opacity = ground.opacity;
            params.ground_shadow_strength = ground.shadow_strength;
            params.ground_reflection = ground.reflection;
            params.ground_roughness = ground.roughness;
        } else {
            params.shadow_receiver_enabled = false;
        }
        params.transmission = if params.shading_model == shrimply_render_3d::obj::ShadingModel::Pbr
        {
            objects
                .iter()
                .map(|(_, scene)| scene.material.transmission)
                .fold(0.0, f32::max)
        } else {
            0.0
        };
        let scene_identity = shrimply_render_3d::SceneIdentity::for_objects(&scene_objects);
        if self
            .composed
            .as_ref()
            .is_none_or(|session| session.identity() != &scene_identity)
        {
            self.composed = Some(
                shrimply_render_3d::ObjRenderSession::compose(&scene_objects)
                    .map_err(|error| error.to_string())?,
            );
        }
        let session = self
            .composed
            .as_mut()
            .expect("composed 3D scene is available");
        params.model_position = session.mesh().source_center;
        params.model_anchor = glam::Vec3::ZERO;
        params.model_rotation_degrees = glam::Vec3::ZERO;
        params.model_rotation_order = shrimply_scene_3d::RotationOrder::Xyz;
        params.model_scale = glam::Vec3::splat(session.mesh().source_radius);
        params.render_quality = if request.accuracy.content_accurate() {
            shrimply_render_3d::obj::RenderQuality::Final
        } else {
            shrimply_render_3d::obj::RenderQuality::Interactive
        };
        let canvas_size = request.render_canvas;
        let width = canvas_size.width.max(1);
        let height = canvas_size.height.max(1);
        let uniforms = params
            .uniforms(width, height)
            .map_err(|error| error.to_string())?;
        let environment = params
            .environment_file
            .as_ref()
            .map(Asset::snapshot)
            .transpose()
            .map_err(|error| error.to_string())?;
        let cached = self.cached.as_ref().filter(|cached| {
            let mut cached_uniforms = cached.uniforms;
            let cached_quality = cached_uniforms.pbr.render_quality;
            let requested_quality = uniforms.pbr.render_quality;
            let quality_satisfies = match requested_quality {
                shrimply_render_3d::obj::RenderQuality::Interactive => true,
                shrimply_render_3d::obj::RenderQuality::Final => matches!(
                    cached_quality,
                    shrimply_render_3d::obj::RenderQuality::Final
                ),
            };
            cached_uniforms.pbr.render_quality = uniforms.pbr.render_quality;
            request.transmission_background.is_none()
                && cached.renderer_generation == compositor.generated_renderer_generation()
                && cached.width == width
                && cached.height == height
                && cached.environment == environment
                && cached.scene == scene_identity
                && quality_satisfies
                && cached_uniforms == uniforms
        });
        let layer = if let Some(cached) = cached {
            cached.layer.clone()
        } else {
            let layer = Rc::new(compositor.render_scene_3d(
                session,
                width,
                height,
                &params,
                request.transmission_background,
            )?);
            if request.transmission_background.is_none() {
                self.cached = Some(CachedObjFrame {
                    renderer_generation: compositor.generated_renderer_generation(),
                    width,
                    height,
                    environment,
                    scene: session.identity().clone(),
                    uniforms,
                    layer: layer.clone(),
                });
            }
            layer
        };
        Ok(VisualRender::Ready(Visual::Raster(
            RasterVisual::materialized(GpuFrame::Rgba(layer), request.state.baked()),
        )))
    }
}

fn text_geometry_key(geometry: &shrimply_text_3d::Geometry<'_>) -> u64 {
    let mut key = DefaultHasher::new();
    geometry.text.hash(&mut key);
    geometry.font_families.hash(&mut key);
    geometry.font_style.hash(&mut key);
    for variation in geometry.font_variations {
        variation.axis.hash(&mut key);
        variation.value.to_bits().hash(&mut key);
    }
    geometry.font_weight.to_bits().hash(&mut key);
    geometry.h_align.hash(&mut key);
    geometry.v_align.hash(&mut key);
    geometry.direction.hash(&mut key);
    geometry.font_size.to_bits().hash(&mut key);
    geometry.depth.to_bits().hash(&mut key);
    geometry.roundness.to_bits().hash(&mut key);
    geometry.smoothness.to_bits().hash(&mut key);
    key.finish()
}

fn shape_geometry_key(geometry: shrimply_shape_3d::Geometry) -> u64 {
    let mut key = DefaultHasher::new();
    geometry.shape.hash(&mut key);
    for value in geometry.size.to_array() {
        value.to_bits().hash(&mut key);
    }
    geometry.corner_radius.to_bits().hash(&mut key);
    geometry.rounding_strategy.hash(&mut key);
    geometry.edge_roundness.to_bits().hash(&mut key);
    geometry.smoothness.to_bits().hash(&mut key);
    geometry.star_points.to_bits().hash(&mut key);
    geometry.star_inner_radius_percent.to_bits().hash(&mut key);
    geometry.arrow_shaft_width_percent.to_bits().hash(&mut key);
    geometry.arrow_head_length_percent.to_bits().hash(&mut key);
    geometry
        .cross_arm_thickness_percent
        .to_bits()
        .hash(&mut key);
    geometry.disk_inner_radius_percent.to_bits().hash(&mut key);
    geometry.disk_completion_degrees.to_bits().hash(&mut key);
    geometry.torus_inner_radius_percent.to_bits().hash(&mut key);
    key.finish()
}
