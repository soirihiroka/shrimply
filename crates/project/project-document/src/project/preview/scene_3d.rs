use glam::{Quat, Vec2, Vec3};
use shrimply_3d_control_core::{
    Axis, CameraDrag, Control, ControlInput, DragOptions, Edit, Handle, Plane, ResolvedTransform3D,
};
use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, Paint, PointerButton, PointerEvent, PreviewBuilder,
    PreviewContext, PreviewEditOutcome, PreviewEditSink, PreviewExtensionKey, PreviewProvider,
    PreviewRefresh, PreviewResponse, PreviewTarget, Stroke,
};
use shrimply_visual_modifiers::{
    MODIFIER_PREVIEW_FACET, ModifierEffect,
    scene_3d::{GroundKind, Scene3dModifierEffect},
};

use super::super::{VisualItem, VisualSource};

const POSITION_RADIUS: f32 = 7.0;
const ARROW_LENGTH: f32 = 48.0;
const ROTATION_RADIUS: f32 = 55.0;
const SCALE_RADIUS: f32 = 68.0;
const HIT_WIDTH: f32 = 8.0;
const LINE_WIDTH: f32 = 2.5;
const SHADOW_WIDTH: f32 = 5.0;
const WORLD_MARGIN: f32 = 48.0;
const WORLD_AXIS_LENGTH: f32 = 28.0;
const WORLD_RADIUS: f32 = 40.0;

pub const TRACKED_CAMERA_PREVIEW: PreviewExtensionKey =
    PreviewExtensionKey::new("scene-3d.tracked-camera");

#[derive(Clone, Copy, Debug)]
pub struct TrackedCameraPreview {
    pub position: Vec3,
    pub rotation: Quat,
    pub projection: shrimply_3d_control_core::Projection,
    pub vertical_fov_degrees: f32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TargetKind {
    Gaussian,
    Model,
    Ground,
    PointLight,
    SunLight,
}

#[derive(Clone, Copy)]
struct Entry {
    target: PreviewTarget,
    kind: TargetKind,
    control: Control,
    model: ResolvedTransform3D,
    ground_size: Option<f32>,
}

#[derive(Clone, Copy)]
enum DragKind {
    Gizmo { entry: usize, handle: Handle },
    Camera(CameraDrag),
}

#[derive(Clone, Copy)]
struct Drag {
    kind: DragKind,
    control: Control,
    start_canvas: Vec2,
    changed: bool,
}

struct SceneHandler {
    item_target: PreviewTarget,
    snapshot: VisualItem,
    navigation: Control,
    navigation_model: ResolvedTransform3D,
    camera: Camera,
    tracked_camera: Option<TrackedCameraPreview>,
    canvas: Vec2,
    entries: Vec<Entry>,
    hovered: Option<(usize, Handle)>,
    drag: Option<Drag>,
}

pub(super) fn provider(
    item: &VisualItem,
    target: PreviewTarget,
    builder: &impl PreviewBuilder,
) -> Option<Box<dyn PreviewProvider>> {
    let canvas = builder.viewport().canvas_size;
    let (mut camera, navigation_model, gaussian) = match &item.content {
        VisualSource::Obj(scene) => (
            Camera {
                projection: scene.camera.projection,
                position: builder.resolve(&scene.camera.position),
                rotation: builder.resolve(&scene.camera.rotation_degrees),
                vertical_fov: builder.resolve(&scene.camera.vertical_fov_degrees),
                orthographic_height: builder.resolve(&scene.camera.orthographic_height),
            },
            resolved_transform(&scene.model, builder),
            false,
        ),
        VisualSource::Gaussian(scene) => (
            Camera {
                projection: scene.camera.projection,
                position: builder.resolve(&scene.camera.position),
                rotation: builder.resolve(&scene.camera.rotation_degrees),
                vertical_fov: builder.resolve(&scene.camera.vertical_fov_degrees),
                orthographic_height: builder.resolve(&scene.camera.orthographic_height),
            },
            resolved_gaussian_transform(&scene.model, builder),
            true,
        ),
        _ => return None,
    };
    let tracked_camera = builder
        .extension(target, TRACKED_CAMERA_PREVIEW)
        .and_then(|value| value.downcast_ref::<TrackedCameraPreview>())
        .copied();
    if let Some(tracked) = tracked_camera {
        camera = camera.with_tracking(tracked);
    }
    let mut navigation = control(navigation_model, camera, canvas)?;
    if gaussian {
        navigation = navigation.keep_camera_outside(navigation_model.scale.abs().max_element());
    }
    let item_target = PreviewTarget::new(item.id, super::ITEM_PREVIEW_FACET);
    let focused_modifier = (target.owner_id() != item.id).then_some(target.owner_id());
    let mut entries = Vec::new();
    if gaussian {
        entries.push(Entry {
            target: item_target,
            kind: TargetKind::Gaussian,
            control: navigation,
            model: navigation_model,
            ground_size: None,
        });
    } else {
        for modifier in item.modifiers.iter().filter(|modifier| modifier.enabled) {
            if focused_modifier.is_some_and(|focused| focused != modifier.id) {
                continue;
            }
            let ModifierEffect::Scene3d(effect) = &modifier.effect else {
                continue;
            };
            let entry_target = PreviewTarget::new(modifier.id, MODIFIER_PREVIEW_FACET);
            let (kind, model) = match &**effect {
                Scene3dModifierEffect::Object(value) if value.file.is_some() => (
                    TargetKind::Model,
                    resolved_transform(&value.transform, builder),
                ),
                Scene3dModifierEffect::Text(value) => (
                    TargetKind::Model,
                    resolved_transform(&value.transform, builder),
                ),
                Scene3dModifierEffect::Shape(value) => (
                    TargetKind::Model,
                    resolved_transform(&value.transform, builder),
                ),
                Scene3dModifierEffect::Ground(value) => (
                    TargetKind::Ground,
                    ResolvedTransform3D {
                        position: builder.resolve(&value.position),
                        rotation_degrees: builder.resolve(&value.rotation_degrees),
                        ..Default::default()
                    },
                ),
                Scene3dModifierEffect::PointLight(value) => (
                    TargetKind::PointLight,
                    ResolvedTransform3D {
                        position: builder.resolve(&value.position),
                        ..Default::default()
                    },
                ),
                Scene3dModifierEffect::SunLight(value) => (
                    TargetKind::SunLight,
                    ResolvedTransform3D {
                        position: navigation_model.position,
                        rotation_degrees: builder.resolve(&value.rotation_degrees),
                        ..Default::default()
                    },
                ),
                _ => continue,
            };
            let mut entry_control = control(model, camera, canvas)?;
            if kind == TargetKind::SunLight {
                let scale = builder.viewport().content_rect.width() / canvas.x.max(1.0);
                let margin = (SCALE_RADIUS + 20.0) / scale.max(f32::EPSILON);
                entry_control =
                    entry_control.with_canvas_anchor(Vec2::new(canvas.x - margin, margin));
            }
            let ground_size = match &**effect {
                Scene3dModifierEffect::Ground(value) if value.kind == GroundKind::Square => {
                    Some(builder.resolve(&value.size).max(f32::EPSILON))
                }
                _ => None,
            };
            entries.push(Entry {
                target: entry_target,
                kind,
                control: entry_control,
                model,
                ground_size,
            });
        }
    }
    Some(Box::new(SceneHandler {
        item_target,
        snapshot: item.clone(),
        navigation,
        navigation_model,
        camera,
        tracked_camera,
        canvas,
        entries,
        hovered: None,
        drag: None,
    }))
}

#[derive(Clone, Copy)]
struct Camera {
    projection: shrimply_3d_control_core::Projection,
    position: Vec3,
    rotation: Vec3,
    vertical_fov: f32,
    orthographic_height: f32,
}

impl Camera {
    fn with_tracking(self, tracked: TrackedCameraPreview) -> Self {
        let custom_rotation = shrimply_transform_3d::rotation(
            self.rotation,
            shrimply_transform_3d::RotationOrder::Xyz,
        );
        Self {
            projection: tracked.projection,
            position: self.position + custom_rotation * tracked.position,
            rotation: shrimply_transform_3d::rotation_degrees(
                custom_rotation * tracked.rotation,
                shrimply_transform_3d::RotationOrder::Xyz,
            ),
            vertical_fov: tracked.vertical_fov_degrees,
            ..self
        }
    }
}

fn resolved_transform(
    transform: &shrimply_scene_3d::Transform3d,
    builder: &impl PreviewBuilder,
) -> ResolvedTransform3D {
    ResolvedTransform3D {
        position: builder.resolve(&transform.position),
        anchor: builder.resolve(&transform.anchor),
        rotation_degrees: builder.resolve(&transform.rotation_degrees),
        rotation_order: builder.resolve(&transform.rotation_order),
        scale: builder.resolve(&transform.scale),
    }
}

fn resolved_gaussian_transform(
    transform: &shrimply_3dgs_core::AnimatedTransform3d,
    builder: &impl PreviewBuilder,
) -> ResolvedTransform3D {
    ResolvedTransform3D {
        position: builder.resolve(&transform.position),
        anchor: builder.resolve(&transform.anchor),
        rotation_degrees: builder.resolve(&transform.rotation_degrees),
        rotation_order: builder.resolve(&transform.rotation_order),
        scale: builder.resolve(&transform.scale),
    }
}

fn control(model: ResolvedTransform3D, camera: Camera, canvas: Vec2) -> Option<Control> {
    Control::new(ControlInput {
        model,
        camera_position: camera.position,
        camera_rotation_degrees: camera.rotation,
        projection: camera.projection,
        vertical_fov_degrees: camera.vertical_fov,
        orthographic_height: camera.orthographic_height,
        canvas_size: canvas,
    })
}

fn square_border(
    transform: ResolvedTransform3D,
    size: f32,
    camera: Camera,
    canvas: Vec2,
) -> Option<[Vec2; 4]> {
    let rotation =
        shrimply_transform_3d::rotation(transform.rotation_degrees, transform.rotation_order);
    let half = size * 0.5;
    let corners = [
        Vec3::new(-half, 0.0, -half),
        Vec3::new(half, 0.0, -half),
        Vec3::new(half, 0.0, half),
        Vec3::new(-half, 0.0, half),
    ];
    let mut result = [Vec2::ZERO; 4];
    for (result, corner) in result.iter_mut().zip(corners) {
        *result = control(
            ResolvedTransform3D {
                position: transform.position + rotation * corner,
                ..Default::default()
            },
            camera,
            canvas,
        )?
        .anchor();
    }
    Some(result)
}

impl SceneHandler {
    fn screen_scale(&self, context: &dyn PreviewContext) -> f32 {
        context.viewport().content_rect.width() / context.viewport().canvas_size.x.max(1.0)
    }

    fn hit(&self, point: Vec2, context: &dyn PreviewContext) -> Option<(usize, Handle)> {
        let scale = self.screen_scale(context);
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, entry)| {
                let anchor = context
                    .viewport()
                    .canvas_point_to_screen(entry.control.anchor());
                let gizmo = entry
                    .control
                    .gizmo(scale, ARROW_LENGTH, ROTATION_RADIUS, SCALE_RADIUS);
                let offset = point - anchor;
                let handle = match entry.kind {
                    TargetKind::Gaussian | TargetKind::Model => {
                        gizmo.hit(offset, POSITION_RADIUS, HIT_WIDTH)
                    }
                    TargetKind::Ground => gizmo
                        .hit_position(offset, POSITION_RADIUS, HIT_WIDTH)
                        .or_else(|| gizmo.hit_rotation(offset, HIT_WIDTH)),
                    TargetKind::PointLight => {
                        gizmo.hit_position(offset, POSITION_RADIUS, HIT_WIDTH)
                    }
                    TargetKind::SunLight => gizmo.hit_rotation(offset, HIT_WIDTH),
                }?;
                Some((index, handle))
            })
    }

    fn apply_gizmo(
        &mut self,
        entry_index: usize,
        handle: Handle,
        point: Vec2,
        modifiers: shrimply_preview_provider_skia::Modifiers,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let entry = self.entries[entry_index];
        let Some(drag) = self.drag else { return false };
        let current = context.viewport().screen_point_to_canvas(point);
        let Some(edit) = drag.control.drag(
            handle,
            current - drag.start_canvas,
            drag.start_canvas - drag.control.anchor(),
            DragOptions {
                constrain_axis: modifiers
                    .contains(shrimply_preview_provider_skia::Modifiers::SHIFT),
            },
        ) else {
            return false;
        };
        let time = edits.keyframe_time();
        let changed = match entry.kind {
            TargetKind::Gaussian => {
                let item = edits
                    .target_mut(entry.target)
                    .downcast_mut::<VisualItem>()
                    .expect("3D item preview target has the wrong type");
                let VisualSource::Gaussian(scene) = &mut item.content else {
                    panic!("Gaussian preview target is not Gaussian");
                };
                set_transform_edit(&mut scene.model, time, edit)
            }
            _ => {
                let effect = edits
                    .target_mut(entry.target)
                    .downcast_mut::<Scene3dModifierEffect>()
                    .expect("3D modifier preview target has the wrong type");
                set_modifier_edit(effect, entry.kind, time, edit)
            }
        };
        if changed && let Some(drag) = &mut self.drag {
            drag.changed = true;
        }
        if changed {
            match edit {
                Edit::Position(value) => self.entries[entry_index].model.position = value,
                Edit::Rotation(value) => self.entries[entry_index].model.rotation_degrees = value,
                Edit::Scale(value) => self.entries[entry_index].model.scale = value,
            }
            if let Some(next) = control(self.entries[entry_index].model, self.camera, self.canvas) {
                let anchor = self.entries[entry_index].control.anchor();
                self.entries[entry_index].control = if entry.kind == TargetKind::SunLight {
                    next.with_canvas_anchor(anchor)
                } else {
                    next
                };
            }
        }
        changed
    }

    fn apply_camera(
        &mut self,
        kind: CameraDrag,
        point: Vec2,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let Some(drag) = self.drag else { return false };
        let current = context.viewport().screen_point_to_canvas(point);
        let Some(edit) = drag.control.drag_camera(kind, current - drag.start_canvas) else {
            return false;
        };
        let time = edits.keyframe_time();
        let item = edits
            .target_mut(self.item_target)
            .downcast_mut::<VisualItem>()
            .expect("3D item preview target has the wrong type");
        let (position, rotation) = if let Some(tracked) = self.tracked_camera {
            let desired_rotation = shrimply_transform_3d::rotation(
                edit.rotation_degrees,
                shrimply_transform_3d::RotationOrder::Xyz,
            );
            let custom_rotation = desired_rotation * tracked.rotation.inverse();
            (
                edit.position - custom_rotation * tracked.position,
                shrimply_transform_3d::rotation_degrees(
                    custom_rotation,
                    shrimply_transform_3d::RotationOrder::Xyz,
                ),
            )
        } else {
            (edit.position, edit.rotation_degrees)
        };
        let changed = match &mut item.content {
            VisualSource::Obj(scene) => {
                super::visual_item::set_vec3(&mut scene.camera.position, time, position)
                    | super::visual_item::set_vec3(
                        &mut scene.camera.rotation_degrees,
                        time,
                        rotation,
                    )
            }
            VisualSource::Gaussian(scene) => {
                super::visual_item::set_vec3(&mut scene.camera.position, time, position)
                    | super::visual_item::set_vec3(
                        &mut scene.camera.rotation_degrees,
                        time,
                        rotation,
                    )
            }
            _ => panic!("3D camera preview target is not a 3D scene"),
        };
        if changed && let Some(drag) = &mut self.drag {
            drag.changed = true;
        }
        if changed {
            self.camera.position = edit.position;
            self.camera.rotation = edit.rotation_degrees;
            if let Some(navigation) = control(self.navigation_model, self.camera, self.canvas) {
                self.navigation = navigation;
            }
            for entry in &mut self.entries {
                let anchor = entry.control.anchor();
                if let Some(next) = control(entry.model, self.camera, self.canvas) {
                    entry.control = if entry.kind == TargetKind::SunLight {
                        next.with_canvas_anchor(anchor)
                    } else {
                        next
                    };
                }
            }
        }
        changed
    }
}

impl PreviewProvider for SceneHandler {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        draw_world(painter, self.navigation, context);
        let active = self.drag.and_then(|drag| match drag.kind {
            DragKind::Gizmo { entry, handle } => Some((entry, handle)),
            DragKind::Camera(_) => None,
        });
        for (index, entry) in self.entries.iter().enumerate() {
            if let Some(border) = entry
                .ground_size
                .and_then(|size| square_border(entry.model, size, self.camera, self.canvas))
            {
                let points = border.map(|point| context.viewport().canvas_point_to_screen(point));
                shrimply_preview_provider_skia::drawing::polyline(
                    painter,
                    &points,
                    true,
                    Paint::stroke(Stroke::new(
                        shrimply_preview_provider_skia::selection_color(),
                        LINE_WIDTH,
                    )),
                );
            }
            if active.is_none_or(|(active_entry, _)| active_entry == index) {
                let visible = active.map(|(_, handle)| handle);
                let highlighted = visible.or_else(|| {
                    self.hovered
                        .filter(|(hovered_entry, _)| *hovered_entry == index)
                        .map(|(_, handle)| handle)
                });
                draw_entry(painter, *entry, visible, highlighted, context);
            }
        }
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        match event {
            PointerEvent::Hover(input) => {
                let hit = self.hit(input.sample.position, context);
                let redraw = self.hovered != hit;
                self.hovered = hit;
                PreviewResponse {
                    handled: hit.is_some(),
                    redraw,
                    cursor: hit.map_or(CursorUpdate::Clear, |(_, handle)| {
                        CursorUpdate::Set(handle_cursor(handle, false))
                    }),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) => {
                let start_canvas = context
                    .viewport()
                    .screen_point_to_canvas(input.sample.position);
                let (kind, control) = if input.button == PointerButton::Middle {
                    (
                        DragKind::Camera(
                            if input
                                .modifiers
                                .contains(shrimply_preview_provider_skia::Modifiers::CONTROL)
                            {
                                CameraDrag::Dolly
                            } else if input
                                .modifiers
                                .contains(shrimply_preview_provider_skia::Modifiers::SHIFT)
                            {
                                CameraDrag::Pan
                            } else {
                                CameraDrag::Orbit
                            },
                        ),
                        self.navigation,
                    )
                } else {
                    let Some((entry, handle)) = self.hit(input.sample.position, context) else {
                        return PreviewResponse::IGNORED;
                    };
                    (
                        DragKind::Gizmo { entry, handle },
                        self.entries[entry].control,
                    )
                };
                self.drag = Some(Drag {
                    kind,
                    control,
                    start_canvas,
                    changed: false,
                });
                self.hovered = None;
                PreviewResponse {
                    handled: true,
                    redraw: true,
                    cursor: CursorUpdate::Set(Cursor::Grabbing),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Samples { input, samples } => {
                let point = samples
                    .last()
                    .map_or(input.sample.position, |sample| sample.position);
                let changed = match self.drag.map(|drag| drag.kind) {
                    Some(DragKind::Gizmo { entry, handle }) => {
                        self.apply_gizmo(entry, handle, point, input.modifiers, context, edits)
                    }
                    Some(DragKind::Camera(kind)) => self.apply_camera(kind, point, context, edits),
                    None => false,
                };
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::live(PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR)
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            PointerEvent::End(input) => {
                let changed = self.drag.take().is_some_and(|drag| drag.changed);
                let hit = self.hit(input.sample.position, context);
                self.hovered = hit;
                PreviewResponse {
                    handled: true,
                    redraw: true,
                    cursor: hit.map_or(CursorUpdate::Clear, |(_, handle)| {
                        CursorUpdate::Set(handle_cursor(handle, false))
                    }),
                    edit: if changed {
                        PreviewEditOutcome::committed(
                            PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                        )
                    } else {
                        PreviewEditOutcome::UNCHANGED
                    },
                }
            }
            PointerEvent::Cancel => {
                let changed = self.drag.take().is_some_and(|drag| drag.changed);
                self.hovered = None;
                if changed {
                    *edits
                        .target_mut(self.item_target)
                        .downcast_mut::<VisualItem>()
                        .expect("3D item preview target has the wrong type") =
                        self.snapshot.clone();
                }
                PreviewResponse {
                    handled: true,
                    redraw: true,
                    cursor: CursorUpdate::Clear,
                    edit: if changed {
                        PreviewEditOutcome::live(
                            PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                        )
                    } else {
                        PreviewEditOutcome::UNCHANGED
                    },
                }
            }
            PointerEvent::Leave => {
                let redraw = self.hovered.take().is_some();
                PreviewResponse {
                    redraw,
                    cursor: CursorUpdate::Clear,
                    ..PreviewResponse::IGNORED
                }
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

fn set_transform_edit(
    transform: &mut shrimply_3dgs_core::AnimatedTransform3d,
    time: super::super::Time,
    edit: Edit,
) -> bool {
    match edit {
        Edit::Position(value) => super::visual_item::set_vec3(&mut transform.position, time, value),
        Edit::Rotation(value) => {
            super::visual_item::set_vec3(&mut transform.rotation_degrees, time, value)
        }
        Edit::Scale(value) => super::visual_item::set_vec3(&mut transform.scale, time, value),
    }
}

fn set_scene_transform_edit(
    transform: &mut shrimply_scene_3d::Transform3d,
    time: super::super::Time,
    edit: Edit,
) -> bool {
    match edit {
        Edit::Position(value) => super::visual_item::set_vec3(&mut transform.position, time, value),
        Edit::Rotation(value) => {
            super::visual_item::set_vec3(&mut transform.rotation_degrees, time, value)
        }
        Edit::Scale(value) => super::visual_item::set_vec3(&mut transform.scale, time, value),
    }
}

fn set_modifier_edit(
    effect: &mut Scene3dModifierEffect,
    kind: TargetKind,
    time: super::super::Time,
    edit: Edit,
) -> bool {
    match (effect, kind, edit) {
        (Scene3dModifierEffect::Object(value), TargetKind::Model, edit) => {
            set_scene_transform_edit(&mut value.transform, time, edit)
        }
        (Scene3dModifierEffect::Text(value), TargetKind::Model, edit) => {
            set_scene_transform_edit(&mut value.transform, time, edit)
        }
        (Scene3dModifierEffect::Shape(value), TargetKind::Model, edit) => {
            set_scene_transform_edit(&mut value.transform, time, edit)
        }
        (Scene3dModifierEffect::Ground(value), TargetKind::Ground, Edit::Position(next)) => {
            super::visual_item::set_vec3(&mut value.position, time, next)
        }
        (Scene3dModifierEffect::Ground(value), TargetKind::Ground, Edit::Rotation(next)) => {
            super::visual_item::set_vec3(&mut value.rotation_degrees, time, next)
        }
        (
            Scene3dModifierEffect::PointLight(value),
            TargetKind::PointLight,
            Edit::Position(next),
        ) => super::visual_item::set_vec3(&mut value.position, time, next),
        (Scene3dModifierEffect::SunLight(value), TargetKind::SunLight, Edit::Rotation(next)) => {
            super::visual_item::set_vec3(&mut value.rotation_degrees, time, next)
        }
        _ => false,
    }
}

fn draw_entry(
    painter: &shrimply_preview_provider_skia::PreviewCanvas,
    entry: Entry,
    visible: Option<Handle>,
    highlighted: Option<Handle>,
    context: &dyn PreviewContext,
) {
    let scale = context.viewport().content_rect.width() / context.viewport().canvas_size.x.max(1.0);
    let anchor = context
        .viewport()
        .canvas_point_to_screen(entry.control.anchor());
    let gizmo = entry
        .control
        .gizmo(scale, ARROW_LENGTH, ROTATION_RADIUS, SCALE_RADIUS);
    if matches!(entry.kind, TargetKind::Gaussian | TargetKind::Model)
        && handle_visible(visible, Handle::Scale)
    {
        let handle = Handle::Scale;
        shrimply_preview_provider_skia::drawing::circle(
            painter,
            anchor,
            gizmo.scale_radius,
            Paint::stroke(Stroke::new(
                handle_color(
                    Color::new(0.92, 0.92, 0.92, 0.55),
                    highlighted == Some(handle),
                ),
                LINE_WIDTH,
            )),
        );
    }
    if !matches!(entry.kind, TargetKind::PointLight) {
        for axis in &gizmo.axes {
            let handle = Handle::Rotation(axis.axis);
            if !handle_visible(visible, handle) {
                continue;
            }
            let points = axis
                .rotation
                .iter()
                .map(|point| anchor + *point)
                .collect::<Vec<_>>();
            shrimply_preview_provider_skia::drawing::polyline(
                painter,
                &points,
                false,
                Paint::stroke(Stroke::new(
                    handle_color(axis_color(axis.axis, 0.55), highlighted == Some(handle)),
                    LINE_WIDTH,
                )),
            );
        }
    }
    if !matches!(entry.kind, TargetKind::SunLight) {
        for plane in &gizmo.planes {
            let handle = Handle::PositionPlane(plane.plane);
            if !handle_visible(visible, handle) {
                continue;
            }
            let points = plane.corners.map(|point| anchor + point);
            shrimply_preview_provider_skia::drawing::polyline(
                painter,
                &points,
                true,
                Paint::fill(handle_fill_color(
                    plane_color(plane.plane, 0.22),
                    highlighted == Some(handle),
                )),
            );
        }
        for axis in &gizmo.axes {
            let handle = Handle::PositionAxis(axis.axis);
            if !handle_visible(visible, handle) || axis.arrow.length_squared() <= f32::EPSILON {
                continue;
            }
            let color = handle_color(axis_color(axis.axis, 0.85), highlighted == Some(handle));
            let end = anchor + axis.arrow;
            shrimply_preview_provider_skia::drawing::line(
                painter,
                anchor,
                end,
                Stroke::new(Color::new(0.0, 0.0, 0.0, 0.55), SHADOW_WIDTH),
            );
            shrimply_preview_provider_skia::drawing::line(
                painter,
                anchor,
                end,
                Stroke::new(color, LINE_WIDTH),
            );
            let direction = axis.arrow.normalize();
            let side = Vec2::new(-direction.y, direction.x) * 5.0;
            let base = end - direction * 9.0;
            shrimply_preview_provider_skia::drawing::polyline(
                painter,
                &[end, base + side, base - side],
                true,
                Paint::fill(color),
            );
        }
    }
    if handle_visible(visible, Handle::Position) {
        shrimply_preview_provider_skia::drawing::circle(
            painter,
            anchor,
            POSITION_RADIUS + 2.0,
            Paint::fill(Color::new(0.0, 0.0, 0.0, 0.55)),
        );
        let color = if matches!(entry.kind, TargetKind::PointLight | TargetKind::SunLight) {
            Color::new(1.0, 0.8, 0.29, 0.9)
        } else {
            shrimply_preview_provider_skia::selection_color()
        };
        shrimply_preview_provider_skia::drawing::circle(
            painter,
            anchor,
            POSITION_RADIUS,
            Paint::fill(handle_color(color, highlighted == Some(Handle::Position))),
        );
    }
}

fn handle_visible(visible: Option<Handle>, handle: Handle) -> bool {
    visible.is_none_or(|visible| visible == handle)
}

fn handle_color(color: Color, highlighted: bool) -> Color {
    if highlighted { Color::YELLOW3 } else { color }
}

fn handle_fill_color(color: Color, highlighted: bool) -> Color {
    if highlighted {
        Color::YELLOW3.with_alpha(0.45)
    } else {
        color
    }
}

fn draw_world(
    painter: &shrimply_preview_provider_skia::PreviewCanvas,
    control: Control,
    context: &dyn PreviewContext,
) {
    let rect = context.viewport().content_rect;
    let margin = WORLD_MARGIN
        .min(rect.width() * 0.5)
        .min(rect.height() * 0.5);
    let anchor = Vec2::new(rect.left() + margin, rect.bottom() - margin);
    shrimply_preview_provider_skia::drawing::circle(
        painter,
        anchor,
        WORLD_RADIUS,
        Paint::fill(Color::new(0.06, 0.06, 0.07, 0.55)),
    );
    let gizmo = control.world_coordinate_gizmo(WORLD_AXIS_LENGTH);
    for axis in &gizmo.axes {
        for (endpoint, alpha, positive) in
            [(axis.endpoint, 0.85, true), (-axis.endpoint, 0.35, false)]
        {
            let end = anchor + endpoint;
            shrimply_preview_provider_skia::drawing::line(
                painter,
                anchor,
                end,
                Stroke::new(axis_color(axis.axis, alpha), 2.0),
            );
            shrimply_preview_provider_skia::drawing::circle(
                painter,
                end,
                8.0,
                Paint::fill(axis_color(axis.axis, if positive { 1.0 } else { 0.45 })),
            );
            if positive {
                shrimply_preview_provider_skia::drawing::text(
                    painter,
                    end,
                    match axis.axis {
                        Axis::X => "X",
                        Axis::Y => "Y",
                        Axis::Z => "Z",
                    },
                    10.0,
                    Color::new(0.08, 0.08, 0.09, 1.0),
                );
            }
        }
    }
}

fn axis_color(axis: Axis, alpha: f32) -> Color {
    match axis {
        Axis::X => Color::new(0.94, 0.27, 0.33, alpha),
        Axis::Y => Color::new(0.39, 0.75, 0.31, alpha),
        Axis::Z => Color::new(0.24, 0.50, 1.0, alpha),
    }
}

fn plane_color(plane: Plane, alpha: f32) -> Color {
    match plane {
        Plane::Xy => axis_color(Axis::Z, alpha),
        Plane::Yz => axis_color(Axis::X, alpha),
        Plane::Zx => axis_color(Axis::Y, alpha),
    }
}

fn handle_cursor(handle: Handle, dragging: bool) -> Cursor {
    match handle {
        Handle::Position | Handle::PositionAxis(_) | Handle::PositionPlane(_) => Cursor::Move,
        Handle::Rotation(_) | Handle::Scale => {
            if dragging {
                Cursor::Grabbing
            } else {
                Cursor::Grab
            }
        }
    }
}
