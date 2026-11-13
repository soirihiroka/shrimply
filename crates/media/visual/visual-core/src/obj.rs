use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

use hashbrown::HashMap;
use shrimply_asset::Asset;
use shrimply_project_document::project::{CanvasSize, VideoItem, VideoItemContent};
use shrimply_visual_modifiers::{ModifierEffect, scene_3d::Scene3dModifierEffect};
use uuid::Uuid;

pub struct State {
    sessions: HashMap<Asset, shrimply_render_3d_core::ObjRenderSession>,
    text_sessions: HashMap<Uuid, TextSession>,
    shape_sessions: HashMap<Uuid, ShapeSession>,
    composed: Option<Arc<shrimply_render_3d_core::ObjRenderSession>>,
    expressions: shrimply_project_evaluation::TransformExpressionCache,
}

struct TextSession {
    key: u64,
    session: shrimply_render_3d_core::ObjRenderSession,
}

struct ShapeSession {
    key: u64,
    session: shrimply_render_3d_core::ObjRenderSession,
}

enum ObjectSource {
    File(Asset),
    Text(Uuid),
    Shape(Uuid),
}

pub struct Request<'a> {
    pub project: &'a shrimply_project_document::project::Project,
    pub item: &'a VideoItem,
    pub position: shrimply_math_core::Time,
    pub audio_analysis: &'a shrimply_project_evaluation::FrameAudioAnalysis,
    pub render_canvas: CanvasSize,
    pub content_accurate: bool,
    pub sequence_path: &'a [Uuid],
    pub track_id: Uuid,
}

pub use shrimply_render_3d_core::PreparedFrame as Prepared;

impl Default for State {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            text_sessions: HashMap::new(),
            shape_sessions: HashMap::new(),
            composed: None,
            expressions: Default::default(),
        }
    }
}

impl State {
    pub fn matches(&self, item: &VideoItem) -> bool {
        matches!(&item.content, VideoItemContent::Obj(_))
    }

    pub fn prepare(&mut self, request: Request<'_>) -> Result<Prepared, String> {
        let VideoItemContent::Obj(scene) = &request.item.content else {
            return Err("OBJ renderer received a non-OBJ visual".to_string());
        };
        let evaluation = shrimply_project_evaluation::VisualEvaluation::for_item_with_audio(
            request.project,
            request.item,
            request.position,
            request.audio_analysis,
        );
        let resolved_scene = shrimply_project_evaluation::resolve_obj_scene(
            scene,
            &evaluation,
            &mut self.expressions,
        );
        let mut params = shrimply_render_3d_core::SceneRenderParams::from(&resolved_scene);
        params.grounds.clear();
        params.shadow_receiver_enabled = false;
        let mut objects = Vec::new();
        if let Some(camera) = crate::camera_reconstruction::sample_for_visual(
            request.project,
            request.item,
            request.sequence_path,
            request.track_id,
            request.position,
        )? {
            let (position, rotation) = shrimply_math_geometry::apply_reconstructed_camera_motion(
                camera.position,
                camera.rotation,
                params.camera_position,
                params.camera_rotation_degrees,
            );
            params.camera_position = position;
            params.camera_rotation_degrees = shrimply_transform_3d::rotation_degrees(
                rotation,
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
                    let resolved = shrimply_project_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::File(path), resolved));
                }
                Scene3dModifierEffect::Text(text) => {
                    let content = shrimply_project_evaluation::resolve_text(
                        &text.text,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let font_size = shrimply_project_evaluation::resolve(
                        &text.font_size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let font_weight = shrimply_project_evaluation::resolve(
                        &text.font_weight,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let depth = shrimply_project_evaluation::resolve(
                        &text.depth,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let roundness = shrimply_project_evaluation::resolve(
                        &text.roundness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let smoothness = shrimply_project_evaluation::resolve(
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
                        let key = text_geometry_key(&geometry);
                        self.text_sessions.insert(
                            modifier.id,
                            TextSession {
                                key,
                                session: shrimply_render_3d_core::ObjRenderSession::generated(
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
                    let resolved = shrimply_project_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::Text(modifier.id), resolved));
                }
                Scene3dModifierEffect::Shape(shape) => {
                    let size = shrimply_project_evaluation::resolve(
                        &shape.size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let corner_radius = shrimply_project_evaluation::resolve(
                        &shape.corner_radius,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let edge_roundness = shrimply_project_evaluation::resolve(
                        &shape.edge_roundness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let smoothness = shrimply_project_evaluation::resolve(
                        &shape.smoothness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let star_points = shrimply_project_evaluation::resolve(
                        &shape.star_points,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let star_inner_radius_percent = shrimply_project_evaluation::resolve(
                        &shape.star_inner_radius_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let arrow_shaft_width_percent = shrimply_project_evaluation::resolve(
                        &shape.arrow_shaft_width_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let arrow_head_length_percent = shrimply_project_evaluation::resolve(
                        &shape.arrow_head_length_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let cross_arm_thickness_percent = shrimply_project_evaluation::resolve(
                        &shape.cross_arm_thickness_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let disk_inner_radius_percent = shrimply_project_evaluation::resolve(
                        &shape.disk_inner_radius_percent,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let disk_completion_degrees = shrimply_project_evaluation::resolve(
                        &shape.disk_completion_degrees,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let torus_inner_radius_percent = shrimply_project_evaluation::resolve(
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
                                session: shrimply_render_3d_core::ObjRenderSession::generated(
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
                    let resolved = shrimply_project_evaluation::resolve_obj_scene(
                        &object_scene,
                        &evaluation,
                        &mut self.expressions,
                    );
                    objects.push((ObjectSource::Shape(modifier.id), resolved));
                }
                Scene3dModifierEffect::Ground(ground) => {
                    let intensity = shrimply_project_evaluation::resolve(
                        &ground.intensity,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let position = shrimply_project_evaluation::resolve(
                        &ground.position,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let rotation_degrees = shrimply_project_evaluation::resolve(
                        &ground.rotation_degrees,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let opacity = shrimply_project_evaluation::resolve(
                        &ground.opacity,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let shadow_strength = shrimply_project_evaluation::resolve(
                        &ground.shadow_strength,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let reflection = shrimply_project_evaluation::resolve(
                        &ground.reflection,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let roughness = shrimply_project_evaluation::resolve(
                        &ground.roughness,
                        &evaluation,
                        &mut self.expressions,
                    );
                    let size = shrimply_project_evaluation::resolve(
                        &ground.size,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params.grounds.push(shrimply_render_3d_core::GroundParams {
                        shape: match ground.kind {
                            shrimply_visual_modifiers::scene_3d::GroundKind::Infinite => {
                                shrimply_render_3d_core::GroundShape::Infinite
                            }
                            shrimply_visual_modifiers::scene_3d::GroundKind::Square => {
                                shrimply_render_3d_core::GroundShape::Square
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
                    let color = shrimply_project_evaluation::resolve(
                        &light.color,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params
                        .point_lights
                        .push(shrimply_render_3d_core::PointLightParams {
                            position: shrimply_project_evaluation::resolve(
                                &light.position,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            color_linear: color.to_linear(),
                            intensity: shrimply_project_evaluation::resolve(
                                &light.intensity,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            range: shrimply_project_evaluation::resolve(
                                &light.range,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            radius: shrimply_project_evaluation::resolve(
                                &light.radius,
                                &evaluation,
                                &mut self.expressions,
                            ),
                        });
                }
                Scene3dModifierEffect::SunLight(light) => {
                    let color = shrimply_project_evaluation::resolve(
                        &light.color,
                        &evaluation,
                        &mut self.expressions,
                    );
                    params
                        .sun_lights
                        .push(shrimply_render_3d_core::SunLightParams {
                            rotation_degrees: shrimply_project_evaluation::resolve(
                                &light.rotation_degrees,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            color_linear: color.to_linear(),
                            intensity: shrimply_project_evaluation::resolve(
                                &light.intensity,
                                &evaluation,
                                &mut self.expressions,
                            ),
                            angular_radius_degrees: shrimply_project_evaluation::resolve(
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
                    shrimply_render_3d_core::ObjRenderSession::load(&path)
                        .map_err(|error| error.to_string())?,
                );
            }
        }
        let scene_objects = objects
            .iter()
            .map(|(source, scene)| shrimply_render_3d_core::SceneObject {
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
                material: shrimply_render_3d_core::SurfaceMaterialParams::from(scene),
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
        params.transmission =
            if params.shading_model == shrimply_render_3d_core::obj::ShadingModel::Pbr {
                objects
                    .iter()
                    .map(|(_, scene)| scene.material.transmission)
                    .fold(0.0, f32::max)
            } else {
                0.0
            };
        let scene_identity = shrimply_render_3d_core::SceneIdentity::for_objects(&scene_objects);
        if self
            .composed
            .as_ref()
            .is_none_or(|session| session.identity() != &scene_identity)
        {
            self.composed = Some(Arc::new(
                shrimply_render_3d_core::ObjRenderSession::compose(&scene_objects)
                    .map_err(|error| error.to_string())?,
            ));
        }
        let session = self
            .composed
            .as_ref()
            .expect("composed 3D scene is available");
        params.model_position = session.mesh().source_center;
        params.model_anchor = glam::Vec3::ZERO;
        params.model_rotation_degrees = glam::Vec3::ZERO;
        params.model_rotation_order = shrimply_scene_3d::RotationOrder::Xyz;
        params.model_scale = glam::Vec3::splat(session.mesh().source_radius);
        params.render_quality = if request.content_accurate {
            shrimply_render_3d_core::obj::RenderQuality::Final
        } else {
            shrimply_render_3d_core::obj::RenderQuality::Interactive
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
        Ok(Prepared {
            session: session.clone(),
            params,
            uniforms,
            environment,
            width,
            height,
        })
    }
}

fn text_geometry_key(geometry: &shrimply_text_3d::Geometry<'_>) -> u64 {
    let mut key = DefaultHasher::new();
    shrimply_fonts::native::system_font_epoch()
        .expect("load shared native font map")
        .hash(&mut key);
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
