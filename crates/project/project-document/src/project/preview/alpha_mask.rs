use glam::{Mat3, Vec2};
use shrimply_preview_provider_skia::{
    Cursor, CursorUpdate, Paint, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewFacetKey, PreviewItemGeometry, PreviewProvider, PreviewRefresh,
    PreviewResponse, PreviewTarget, Rect, Stroke, draw_anchor, draw_control_line,
    draw_control_polyline, draw_keypoints, draw_rotation_handle, hit_keypoint,
    rotation_handle_segment,
};

use super::super::{AlphaMaskShape, VisualAlphaMask};

pub const COMPOSITING_FACET: PreviewFacetKey =
    PreviewFacetKey::new("visual-item.compositing-alpha-mask");
pub const MODIFIER_FACET: PreviewFacetKey = PreviewFacetKey::new("modifier.alpha-mask");

const UNIT_MASK_BOUNDS: Rect = Rect {
    min: Vec2::new(-0.5, -0.5),
    max: Vec2::new(0.5, 0.5),
};

pub fn provider(
    mask: &VisualAlphaMask,
    target: PreviewTarget,
    geometry: PreviewItemGeometry,
    builder: &impl PreviewBuilder,
) -> Option<Box<dyn PreviewProvider>> {
    mask.enabled.then(|| {
        Box::new(AlphaMaskHandler {
            target,
            snapshot: mask.clone(),
            geometry,
            shape: mask.shape,
            center: builder.resolve(&mask.center),
            size: builder.resolve(&mask.size).max(Vec2::ZERO),
            rotation_degrees: builder.resolve(&mask.rotation_degrees),
            center_editable: super::visual_item::editable(&mask.center),
            size_editable: super::visual_item::editable(&mask.size),
            rotation_editable: super::visual_item::editable(&mask.rotation_degrees),
            vertices: mask.vertices.clone(),
            drag: None,
        }) as Box<dyn PreviewProvider>
    })
}

#[derive(Clone, Copy)]
enum DragKind {
    Center,
    Size(usize),
    Rotation,
}

#[derive(Clone, Copy)]
struct DragState {
    kind: DragKind,
    start_pointer: Vec2,
    center: Vec2,
    size: Vec2,
    rotation_degrees: f32,
    changed: bool,
}

struct AlphaMaskHandler {
    target: PreviewTarget,
    snapshot: VisualAlphaMask,
    geometry: PreviewItemGeometry,
    shape: AlphaMaskShape,
    center: Vec2,
    size: Vec2,
    rotation_degrees: f32,
    center_editable: bool,
    size_editable: bool,
    rotation_editable: bool,
    vertices: Vec<Vec2>,
    drag: Option<DragState>,
}

impl AlphaMaskHandler {
    fn map(&self, context: &dyn PreviewContext) -> Mat3 {
        context.viewport().canvas_to_screen
            * self.geometry.local_to_canvas
            * Mat3::from_scale_angle_translation(
                self.geometry.source_size * self.size,
                self.rotation_degrees.to_radians(),
                self.geometry.source_size * self.center,
            )
    }

    fn center_screen(&self, context: &dyn PreviewContext) -> Vec2 {
        self.map(context).transform_point2(Vec2::ZERO)
    }

    fn corners(&self, context: &dyn PreviewContext) -> [Vec2; 4] {
        let map = self.map(context);
        [
            Vec2::new(-0.5, -0.5),
            Vec2::new(0.5, -0.5),
            Vec2::new(0.5, 0.5),
            Vec2::new(-0.5, 0.5),
        ]
        .map(|point| map.transform_point2(point))
    }

    fn rotation_segment(&self, context: &dyn PreviewContext) -> (Vec2, Vec2) {
        rotation_handle_segment(
            self.map(context),
            UNIT_MASK_BOUNDS,
            context.viewport().surface_rect,
        )
    }

    fn rotation_handle(&self, context: &dyn PreviewContext) -> Vec2 {
        self.rotation_segment(context).1
    }

    fn hit(&self, point: Vec2, context: &dyn PreviewContext) -> Option<(DragKind, Cursor)> {
        if self.center_editable && hit_keypoint(point, self.center_screen(context)) {
            return Some((DragKind::Center, Cursor::Move));
        }
        if self.rotation_editable && hit_keypoint(point, self.rotation_handle(context)) {
            return Some((DragKind::Rotation, Cursor::Grab));
        }
        if !self.size_editable {
            return None;
        }
        self.corners(context)
            .into_iter()
            .enumerate()
            .find(|(_, corner)| hit_keypoint(point, *corner))
            .map(|(index, _)| (DragKind::Size(index), Cursor::ResizeDiagonalDown))
    }

    fn drag(
        &mut self,
        point: Vec2,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let Some(drag) = self.drag else { return false };
        let time = edits.keyframe_time();
        let mask = edits
            .target_mut(self.target)
            .downcast_mut::<VisualAlphaMask>()
            .expect("alpha mask preview target has the wrong type");
        let item_map = context.viewport().canvas_to_screen * self.geometry.local_to_canvas;
        if !item_map.determinant().is_finite() || item_map.determinant().abs() <= f32::EPSILON {
            return false;
        }
        let local_start = item_map.inverse().transform_point2(drag.start_pointer)
            / self.geometry.source_size.max(Vec2::ONE);
        let local =
            item_map.inverse().transform_point2(point) / self.geometry.source_size.max(Vec2::ONE);
        let mut center = drag.center;
        let mut size = drag.size;
        let mut rotation = drag.rotation_degrees;
        match drag.kind {
            DragKind::Center => center += local - local_start,
            DragKind::Size(index) => {
                let axes = match index {
                    0 => Vec2::new(-1.0, -1.0),
                    1 => Vec2::new(1.0, -1.0),
                    2 => Vec2::ONE,
                    _ => Vec2::new(-1.0, 1.0),
                };
                size = (drag.size + (local - local_start) * axes * 2.0).max(Vec2::ZERO);
            }
            DragKind::Rotation => {
                let center_screen =
                    item_map.transform_point2(self.geometry.source_size * drag.center);
                let start_angle = (drag.start_pointer - center_screen)
                    .y
                    .atan2((drag.start_pointer - center_screen).x);
                let angle = (point - center_screen).y.atan2((point - center_screen).x);
                rotation += (angle - start_angle).to_degrees();
            }
        }
        let changed = super::visual_item::set_vec2(&mut mask.center, time, center)
            | super::visual_item::set_vec2(&mut mask.size, time, size)
            | super::visual_item::set_scalar(&mut mask.rotation_degrees, time, rotation);
        if changed {
            self.center = center;
            self.size = size;
            self.rotation_degrees = rotation;
            if let Some(drag) = &mut self.drag {
                drag.changed = true;
            }
        }
        changed
    }
}

impl PreviewProvider for AlphaMaskHandler {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let foreground = context.selection_foreground_color();
        let map = self.map(context);
        let center = self.center_screen(context);
        let rotation = self
            .rotation_editable
            .then(|| self.rotation_segment(context));
        if let Some((top, knob)) = rotation {
            draw_control_line(painter, top, knob, color);
        }
        if self.shape == AlphaMaskShape::Ellipse {
            let radius = center.distance(map.transform_point2(Vec2::new(0.5, 0.0)));
            shrimply_preview_provider_skia::drawing::circle(
                painter,
                center,
                radius,
                Paint::stroke(Stroke::new(
                    color,
                    shrimply_preview_provider_skia::CONTROL_LINE_WIDTH,
                )),
            );
        } else {
            let points = if self.shape == AlphaMaskShape::Polygon {
                self.vertices
                    .iter()
                    .map(|point| map.transform_point2(*point))
                    .collect::<Vec<_>>()
            } else {
                self.corners(context).to_vec()
            };
            draw_control_polyline(painter, &points, true, color);
        }
        if self.size_editable {
            let corners = self.corners(context);
            draw_keypoints(painter, &corners, color);
        }
        if let Some((_, knob)) = rotation {
            draw_rotation_handle(painter, knob, color, foreground);
        }
        if self.center_editable {
            draw_anchor(painter, center, color);
        }
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        match event {
            shrimply_preview_provider_skia::PointerEvent::Hover(input) => PreviewResponse {
                handled: self.hit(input.sample.position, context).is_some(),
                redraw: false,
                cursor: self
                    .hit(input.sample.position, context)
                    .map_or(CursorUpdate::Clear, |(_, cursor)| CursorUpdate::Set(cursor)),
                edit: PreviewEditOutcome::UNCHANGED,
            },
            shrimply_preview_provider_skia::PointerEvent::Begin(input) => {
                let Some((kind, cursor)) = self.hit(input.sample.position, context) else {
                    return PreviewResponse::IGNORED;
                };
                self.drag = Some(DragState {
                    kind,
                    start_pointer: input.sample.position,
                    center: self.center,
                    size: self.size,
                    rotation_degrees: self.rotation_degrees,
                    changed: false,
                });
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(cursor),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            shrimply_preview_provider_skia::PointerEvent::Samples { input, samples } => {
                let point = samples
                    .last()
                    .map_or(input.sample.position, |sample| sample.position);
                let changed = self.drag(point, context, edits);
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::live(PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR)
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            shrimply_preview_provider_skia::PointerEvent::End(_) => {
                let changed = self.drag.take().is_some_and(|drag| drag.changed);
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::committed(
                        PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                    )
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            shrimply_preview_provider_skia::PointerEvent::Cancel => {
                let changed = self.drag.take().is_some_and(|drag| drag.changed);
                if changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<VisualAlphaMask>()
                        .expect("alpha mask preview target has the wrong type") =
                        self.snapshot.clone();
                }
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::live(PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR)
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}
