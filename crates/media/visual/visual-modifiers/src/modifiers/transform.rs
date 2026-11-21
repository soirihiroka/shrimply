use hashbrown::HashSet;

use serde::{Deserialize, Serialize};
use shrimply_math_geometry::{ResolvedTransform2D, Transform2D};
use uuid::Uuid;

use super::{KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span};
#[cfg(feature = "preview")]
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Modifiers, PointerEvent, PreviewBuilder, PreviewContext,
    PreviewEditOutcome, PreviewEditSink, PreviewProvider, PreviewRefresh, PreviewResponse,
    PreviewTarget,
};
use shrimply_property_model::timeline_value::*;

#[cfg(feature = "preview")]
use super::preview::{self, HANDLES, Handle};

type AnimatedTransform2D = Transform2D<TimelineValue<glam::Vec2>, TimelineValue<f32>>;

/// A complete, animated 2D transform applied after the visual source's transform.
///
/// The modifier owns its transform data. Rendering and inspector code use the focused accessors
/// below instead of knowing how the modifier persists that data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransformModifier {
    transform: AnimatedTransform2D,
}

impl Default for TransformModifier {
    fn default() -> Self {
        Self {
            transform: AnimatedTransform2D::from_resolved(ResolvedTransform2D::IDENTITY),
        }
    }
}

impl TransformModifier {
    pub fn centered_at(center: glam::Vec2) -> Self {
        Self {
            transform: AnimatedTransform2D::from_resolved(ResolvedTransform2D {
                position: center,
                anchor: center,
                ..ResolvedTransform2D::IDENTITY
            }),
        }
    }

    pub fn position(&self) -> &TimelineValue<glam::Vec2> {
        &self.transform.position
    }

    pub fn anchor(&self) -> &TimelineValue<glam::Vec2> {
        &self.transform.anchor
    }

    pub fn scale(&self) -> &TimelineValue<glam::Vec2> {
        &self.transform.scale
    }

    pub fn shear(&self) -> &TimelineValue<glam::Vec2> {
        &self.transform.shear
    }

    pub fn rotation_degrees(&self) -> &TimelineValue<f32> {
        &self.transform.rotation_degrees
    }
}

impl ModifierModel for TransformModifier {
    fn display_name(&self) -> &'static str {
        "Transform"
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        // The wrapper ID identifies this transform as a unit while the value IDs identify its
        // independently animated properties.
        super::ensure_unique_id(&mut self.transform.id, seen);
        ensure_timeline_value_ids(&mut self.transform.position, seen);
        ensure_timeline_value_ids(&mut self.transform.anchor, seen);
        ensure_timeline_value_ids(&mut self.transform.scale, seen);
        ensure_timeline_value_ids(&mut self.transform.shear, seen);
        ensure_timeline_value_ids(&mut self.transform.rotation_degrees, seen);
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine([
            timeline_value_span(&self.transform.position),
            timeline_value_span(&self.transform.anchor),
            timeline_value_span(&self.transform.scale),
            timeline_value_span(&self.transform.shear),
            timeline_value_span(&self.transform.rotation_degrees),
        ])
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        (self.transform.rotation_degrees.id == id).then_some(&self.transform.rotation_degrees)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        (self.transform.rotation_degrees.id == id).then_some(&mut self.transform.rotation_degrees)
    }

    fn number2(&self, id: Uuid) -> Option<&TimelineValue<glam::Vec2>> {
        [
            &self.transform.position,
            &self.transform.anchor,
            &self.transform.scale,
            &self.transform.shear,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number2_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<glam::Vec2>> {
        [
            &mut self.transform.position,
            &mut self.transform.anchor,
            &mut self.transform.scale,
            &mut self.transform.shear,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }
}

#[derive(Clone)]
#[cfg(feature = "preview")]
struct TransformPreview {
    target: PreviewTarget,
    snapshot: TransformModifier,
    screen_map: glam::Mat3,
    canvas_to_screen: glam::Mat3,
    size: glam::Vec2,
    resolved: ResolvedTransform2D,
    editable: [bool; 4],
    active: Option<TransformDrag>,
    changed: bool,
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
enum TransformDragKind {
    Move,
    Anchor,
    Rotate,
    Resize(Handle),
}

#[derive(Clone, Copy)]
#[cfg(feature = "preview")]
struct TransformDrag {
    kind: TransformDragKind,
    canvas_to_screen: glam::Mat3,
    size: glam::Vec2,
    start: ResolvedTransform2D,
    start_pointer_parent: glam::Vec2,
}

impl TransformModifier {
    #[cfg(feature = "preview")]
    pub(crate) fn preview_provider(
        &self,
        target: PreviewTarget,
        builder: &impl PreviewBuilder,
    ) -> Option<Box<dyn PreviewProvider>> {
        if !preview::is_target(target) {
            return None;
        }
        let (screen_map, size) = preview::screen_map(target, builder)?;
        let resolved = ResolvedTransform2D {
            position: builder.resolve(&self.transform.position),
            anchor: builder.resolve(&self.transform.anchor),
            scale: builder.resolve(&self.transform.scale),
            shear: builder.resolve(&self.transform.shear),
            rotation_degrees: builder.resolve(&self.transform.rotation_degrees),
        };
        Some(Box::new(TransformPreview {
            target,
            snapshot: self.clone(),
            screen_map,
            canvas_to_screen: builder.viewport().canvas_to_screen,
            size,
            resolved,
            editable: [
                preview::editable(&self.transform.position),
                preview::editable(&self.transform.anchor),
                preview::editable(&self.transform.scale),
                preview::editable(&self.transform.rotation_degrees),
            ],
            active: None,
            changed: false,
        }))
    }
}

#[cfg(feature = "preview")]
impl TransformPreview {
    fn bounds(&self) -> shrimply_preview_provider_skia::Rect {
        shrimply_preview_provider_skia::Rect::from_min_size(glam::Vec2::ZERO, self.size)
    }

    fn anchor_screen(&self) -> glam::Vec2 {
        self.canvas_to_screen
            .transform_point2(self.resolved.position)
    }

    fn rotation_points(&self) -> (glam::Vec2, glam::Vec2) {
        let bounds = self.bounds();
        let left = self
            .screen_map
            .transform_point2(preview::handle_point(Handle::TopLeft, bounds));
        let right = self
            .screen_map
            .transform_point2(preview::handle_point(Handle::TopRight, bounds));
        let stem = (left + right) * 0.5;
        let edge = right - left;
        let normal = if edge.length_squared() > f32::EPSILON {
            glam::Vec2::new(edge.y, -edge.x).normalize()
        } else {
            -glam::Vec2::Y
        };
        (stem, stem + normal * 28.0)
    }

    fn hit(&self, point: glam::Vec2) -> Option<TransformDragKind> {
        let bounds = self.bounds();
        if self.editable[0] && self.editable[2] {
            for handle in HANDLES {
                if preview::hit(
                    point,
                    self.screen_map
                        .transform_point2(preview::handle_point(handle, bounds)),
                ) {
                    return Some(TransformDragKind::Resize(handle));
                }
            }
        }
        if self.editable[3] && preview::hit(point, self.rotation_points().1) {
            return Some(TransformDragKind::Rotate);
        }
        if self.editable[0] && self.editable[1] && preview::hit(point, self.anchor_screen()) {
            return Some(TransformDragKind::Anchor);
        }
        let local = preview::inverse_point(self.screen_map, point)?;
        (self.editable[0] && bounds.contains(local)).then_some(TransformDragKind::Move)
    }
}

#[cfg(feature = "preview")]
impl PreviewProvider for TransformPreview {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let bounds = self.bounds();
        preview::draw_rect(painter, self.screen_map, bounds, color);
        for handle in HANDLES {
            preview::draw_handle(
                painter,
                self.screen_map
                    .transform_point2(preview::handle_point(handle, bounds)),
                color,
            );
        }
        let (stem, rotation) = self.rotation_points();
        preview::draw_line(painter, stem, rotation, color);
        preview::draw_handle(painter, rotation, color);
        preview::draw_handle(painter, self.anchor_screen(), color);
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        _context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        let time = edits.keyframe_time();
        match event {
            PointerEvent::Hover(input) => {
                let Some(kind) = self.hit(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(transform_cursor(kind)),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Begin(input) => {
                let Some(kind) = self.hit(input.sample.position) else {
                    return PreviewResponse::IGNORED;
                };
                let Some(start_pointer_parent) =
                    preview::inverse_point(self.canvas_to_screen, input.sample.position)
                else {
                    return PreviewResponse::IGNORED;
                };
                self.active = Some(TransformDrag {
                    kind,
                    canvas_to_screen: self.canvas_to_screen,
                    size: self.size,
                    start: self.resolved,
                    start_pointer_parent,
                });
                PreviewResponse::handled()
            }
            PointerEvent::Samples { input, .. } => {
                let Some(drag) = self.active else {
                    return PreviewResponse::IGNORED;
                };
                let Some((changed, resolved)) = update_transform(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<TransformModifier>()
                        .expect("transform preview target has wrong type"),
                    &drag,
                    input.sample.position,
                    input.modifiers,
                    time,
                ) else {
                    return PreviewResponse::IGNORED;
                };
                if changed {
                    self.resolved = resolved;
                }
                self.changed |= changed;
                transform_edit(changed, false)
            }
            PointerEvent::End(_) if self.active.is_some() => {
                self.active = None;
                transform_edit(std::mem::take(&mut self.changed), true)
            }
            PointerEvent::Cancel => {
                if self.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<TransformModifier>()
                        .expect("transform preview target has wrong type") = self.snapshot.clone();
                }
                self.active = None;
                transform_edit(std::mem::take(&mut self.changed), false)
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[cfg(feature = "preview")]
fn transform_cursor(kind: TransformDragKind) -> Cursor {
    match kind {
        TransformDragKind::Resize(handle) => preview::resize_cursor(handle),
        TransformDragKind::Rotate => Cursor::Grab,
        TransformDragKind::Anchor | TransformDragKind::Move => Cursor::Move,
    }
}

#[cfg(feature = "preview")]
fn transform_edit(changed: bool, commit: bool) -> PreviewResponse {
    if !changed {
        return PreviewResponse::handled();
    }
    let refresh = PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR;
    PreviewResponse::edited(if commit {
        PreviewEditOutcome::committed(refresh)
    } else {
        PreviewEditOutcome::live(refresh)
    })
}

#[cfg(feature = "preview")]
fn update_transform(
    modifier: &mut TransformModifier,
    drag: &TransformDrag,
    point: glam::Vec2,
    modifiers: Modifiers,
    time: shrimply_property_model::Time,
) -> Option<(bool, ResolvedTransform2D)> {
    let parent = preview::inverse_point(drag.canvas_to_screen, point)?;
    let start = drag.start;
    let mut resolved = start;
    let changed = match drag.kind {
        TransformDragKind::Move => {
            resolved.position = start.position + parent - drag.start_pointer_parent;
            preview::set_vec2(&mut modifier.transform.position, time, resolved.position)
        }
        TransformDragKind::Anchor => {
            let linear = glam::Mat2::from_cols(
                start.matrix().transform_vector2(glam::Vec2::X),
                start.matrix().transform_vector2(glam::Vec2::Y),
            );
            if linear.determinant().abs() <= f32::EPSILON {
                return None;
            }
            resolved.position = parent;
            resolved.anchor = start.anchor + linear.inverse() * (parent - start.position);
            preview::set_vec2(&mut modifier.transform.position, time, parent)
                | preview::set_vec2(&mut modifier.transform.anchor, time, resolved.anchor)
        }
        TransformDragKind::Rotate => {
            let start_angle = (drag.start_pointer_parent - start.position)
                .y
                .atan2((drag.start_pointer_parent - start.position).x);
            let angle = (parent - start.position)
                .y
                .atan2((parent - start.position).x);
            let delta = (angle - start_angle).to_degrees();
            resolved.rotation_degrees = start.rotation_degrees + delta;
            preview::set_scalar(
                &mut modifier.transform.rotation_degrees,
                time,
                resolved.rotation_degrees,
            )
        }
        TransformDragKind::Resize(handle) => {
            let bounds =
                shrimply_preview_provider_skia::Rect::from_min_size(glam::Vec2::ZERO, drag.size);
            let handle_local = preview::handle_point(handle, bounds);
            let fixed_local = preview::handle_point(opposite(handle), bounds);
            let centered = modifiers.contains(Modifiers::ALT);
            let base_local = if centered { start.anchor } else { fixed_local };
            let base_parent = if centered {
                start.position
            } else {
                start.matrix().transform_point2(fixed_local)
            };
            let rotation_shear = ResolvedTransform2D {
                scale: glam::Vec2::ONE,
                ..start
            }
            .matrix();
            let local_delta = rotation_shear
                .inverse()
                .transform_vector2(parent - base_parent);
            let denominator = handle_local - base_local;
            let (x, y) = preview::handle_axes(handle);
            let mut scale = start.scale;
            if x != 0 && denominator.x.abs() > f32::EPSILON {
                scale.x = (local_delta.x / denominator.x).max(0.001);
            }
            if y != 0 && denominator.y.abs() > f32::EPSILON {
                scale.y = (local_delta.y / denominator.y).max(0.001);
            }
            if modifiers.contains(Modifiers::SHIFT) {
                let factor = if x != 0 {
                    scale.x / start.scale.x
                } else {
                    scale.y / start.scale.y
                };
                scale = start.scale * factor;
            }
            let position = if centered {
                start.position
            } else {
                base_parent
                    - ResolvedTransform2D { scale, ..start }
                        .matrix()
                        .transform_vector2(fixed_local - start.anchor)
            };
            resolved.position = position;
            resolved.scale = scale;
            preview::set_vec2(&mut modifier.transform.position, time, position)
                | preview::set_vec2(&mut modifier.transform.scale, time, scale)
        }
    };
    Some((changed, resolved))
}

#[cfg(feature = "preview")]
fn opposite(handle: Handle) -> Handle {
    match handle {
        Handle::TopLeft => Handle::BottomRight,
        Handle::Top => Handle::Bottom,
        Handle::TopRight => Handle::BottomLeft,
        Handle::Right => Handle::Left,
        Handle::BottomRight => Handle::TopLeft,
        Handle::Bottom => Handle::Top,
        Handle::BottomLeft => Handle::TopRight,
        Handle::Left => Handle::Right,
    }
}
