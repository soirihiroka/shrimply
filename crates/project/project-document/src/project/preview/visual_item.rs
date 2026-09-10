use glam::{Mat3, Vec2, Vec3};
use shrimply_math_geometry::snap::{
    rect_axis_feature, resolve_axis_snap, solve_parameter_correction, transformed_rect,
};
use shrimply_preview_provider_skia::{
    AxisFeatures, AxisGap, AxisSnapKind, BOUNDS_HANDLES, BoundsHandle, Color, Cursor, CursorUpdate,
    PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome, PreviewEditSink,
    PreviewItemGeometry, PreviewProvider, PreviewRefresh, PreviewResponse, PreviewTarget,
    PreviewViewport, Rect, SnapAxis, SnapResult, SnapScene, bounds_handle_position,
    draw_control_line, draw_control_rect, draw_keypoint, draw_keypoints, hit_keypoint,
};
use shrimply_property_model::timeline_value::{
    TimelineBase, TimelineCurveKeyframe, TimelineValue, TimelineValueType,
};
use uuid::Uuid;

use super::super::{ResolvedTransform, Time, VisualItem, VisualSource};

pub const FACET: shrimply_preview_provider_skia::PreviewFacetKey =
    shrimply_preview_provider_skia::PreviewFacetKey::new("visual-item.transform");

const ROTATION_HANDLE_OFFSET: f32 = 28.0;
const ROTATION_SNAP_STEP_DEGREES: f32 = 15.0;
const MIN_SCALE: f32 = 0.001;
const DIRECT_RESIZE_SNAP_PASSES: usize = 4;
const SNAP_MARK_CAP_HALF_LENGTH: f32 = 4.0;
const SNAP_SOLVER_STEP: f32 = 0.001;
const SNAP_RESIDUAL_TOLERANCE: f32 = 0.001;

pub fn geometry(item: &VisualItem, builder: &impl PreviewBuilder) -> Option<PreviewItemGeometry> {
    let rotation_degrees = builder.resolve(&item.transform.rotation_degrees);
    let source_size = match &item.content {
        VisualSource::Shape(shape) => builder.resolve(&shape.size).max(Vec2::ONE),
        VisualSource::Text(text) => builder
            .source_size(item.id)
            .unwrap_or_else(|| super::text::size(text, builder)),
        _ => builder.source_size(item.id)?.max(Vec2::ONE),
    };
    let (bounds, decoration_size, anchor_offset) = match &item.content {
        VisualSource::Text(text) => {
            super::text::bounds(text, source_size, rotation_degrees, builder)
        }
        VisualSource::Shape(shape) => {
            let angle =
                (builder.resolve(&shape.shadow_direction_degrees) - rotation_degrees).to_radians();
            let shadow_offset = Vec2::new(angle.cos(), angle.sin())
                * builder.resolve(&shape.shadow_distance).max(0.0);
            let content = Rect::from_min_size(Vec2::ZERO, source_size);
            let bounds = super::math::decorated_bounds(
                content,
                builder.resolve(&shape.outline_width),
                shadow_offset,
                builder.resolve(&shape.shadow_width),
                builder.resolve(&shape.shadow_blur),
                Vec2::ZERO,
            );
            (bounds, bounds.size() - content.size(), Vec2::ZERO)
        }
        _ => (
            Rect::from_min_size(Vec2::ZERO, source_size),
            Vec2::ZERO,
            Vec2::ZERO,
        ),
    };
    let transform = ResolvedTransform {
        position: builder.resolve(&item.transform.position),
        anchor: builder.resolve(&item.transform.anchor),
        scale: builder.resolve(&item.transform.scale),
        shear: builder.resolve(&item.transform.shear),
        rotation_degrees,
    };
    Some(PreviewItemGeometry {
        source_size,
        bounds,
        decoration_size,
        anchor_offset,
        transform,
        local_to_canvas: transform.matrix(),
    })
}

pub fn provider(
    item: &VisualItem,
    target: PreviewTarget,
    geometry: PreviewItemGeometry,
    builder: &impl PreviewBuilder,
) -> Box<dyn PreviewProvider> {
    let resize = match &item.content {
        VisualSource::Text(text) => ResizeTarget::Text {
            font_size: builder.resolve(&text.font_size).max(1.0),
            editable: editable(&text.font_size),
        },
        VisualSource::Shape(shape) => ResizeTarget::Shape {
            size: builder.resolve(&shape.size).max(Vec2::ONE),
            editable: editable(&shape.size),
        },
        _ => ResizeTarget::Transform,
    };
    Box::new(TransformHandler {
        target: PreviewTarget::new(target.owner_id(), FACET),
        snapshot: None,
        geometry,
        resize,
        position_editable: editable(&item.transform.position),
        anchor_editable: editable(&item.transform.anchor),
        scale_editable: editable(&item.transform.scale),
        rotation_editable: editable(&item.transform.rotation_degrees),
        drag: None,
        snap_scene: None,
        snap_feedback: SnapResult::default(),
    })
}

#[derive(Clone, Copy)]
enum DragKind {
    Move,
    Anchor,
    Resize(BoundsHandle),
    Rotate,
}

#[derive(Clone, Copy)]
struct DragState {
    kind: DragKind,
    free_resize: bool,
    centered_resize: bool,
    start_pointer: Vec2,
    start: ResolvedTransform,
    bounds: Rect,
    source_size: Vec2,
    anchor_offset: Vec2,
    changed: bool,
}

#[derive(Clone, Copy)]
enum ResizeTarget {
    Transform,
    Text { font_size: f32, editable: bool },
    Shape { size: Vec2, editable: bool },
}

struct TransformHandler {
    target: PreviewTarget,
    snapshot: Option<VisualItem>,
    geometry: PreviewItemGeometry,
    resize: ResizeTarget,
    position_editable: bool,
    anchor_editable: bool,
    scale_editable: bool,
    rotation_editable: bool,
    drag: Option<DragState>,
    snap_scene: Option<SnapScene>,
    snap_feedback: SnapResult,
}

impl TransformHandler {
    fn screen_map(&self, context: &dyn PreviewContext) -> Mat3 {
        context.viewport().canvas_to_screen * self.geometry.local_to_canvas
    }

    fn rotation_handle(&self, context: &dyn PreviewContext) -> Vec2 {
        let map = self.screen_map(context);
        let center = map.transform_point2(self.geometry.bounds.center());
        let top = map.transform_point2(Vec2::new(
            self.geometry.bounds.center().x,
            self.geometry.bounds.min.y,
        ));
        top + (top - center).normalize_or_zero() * ROTATION_HANDLE_OFFSET
    }

    fn anchor(&self, context: &dyn PreviewContext) -> Vec2 {
        context
            .viewport()
            .canvas_to_screen
            .transform_point2(self.geometry.transform.position)
    }

    fn resize_editable(&self, control: bool) -> bool {
        if !self.position_editable {
            return false;
        }
        if control {
            return self.scale_editable;
        }
        match self.resize {
            ResizeTarget::Transform => self.scale_editable,
            ResizeTarget::Text { editable, .. } | ResizeTarget::Shape { editable, .. } => editable,
        }
    }

    fn hit(
        &self,
        point: Vec2,
        modifiers: shrimply_preview_provider_skia::Modifiers,
        context: &dyn PreviewContext,
    ) -> Option<(DragKind, Cursor)> {
        let map = self.screen_map(context);
        if self.anchor_editable && hit_keypoint(point, self.anchor(context)) {
            return Some((DragKind::Anchor, Cursor::Grab));
        }
        if self.rotation_editable && hit_keypoint(point, self.rotation_handle(context)) {
            return Some((DragKind::Rotate, Cursor::Grab));
        }
        if self
            .resize_editable(modifiers.contains(shrimply_preview_provider_skia::Modifiers::CONTROL))
        {
            for handle in BOUNDS_HANDLES {
                if hit_keypoint(
                    point,
                    map.transform_point2(bounds_handle_position(handle, self.geometry.bounds)),
                ) {
                    return Some((
                        DragKind::Resize(handle),
                        transformed_resize_cursor(handle, map),
                    ));
                }
            }
        }
        let determinant = map.determinant();
        if self.position_editable
            && determinant.is_finite()
            && determinant.abs() > f32::EPSILON
            && self
                .geometry
                .bounds
                .contains(map.inverse().transform_point2(point))
        {
            Some((DragKind::Move, Cursor::Move))
        } else {
            None
        }
    }

    fn apply_drag(
        &mut self,
        point: Vec2,
        modifiers: shrimply_preview_provider_skia::Modifiers,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let Some(drag) = self.drag else { return false };
        let viewport = context.viewport();
        let screen_to_canvas = viewport.canvas_to_screen.inverse();
        let start_canvas = screen_to_canvas.transform_point2(drag.start_pointer);
        let mut current_canvas = screen_to_canvas.transform_point2(point);
        let delta = current_canvas - start_canvas;
        let mut next = drag.start;
        let mut next_bounds = drag.bounds;
        let mut next_source_size = drag.source_size;
        let mut next_anchor_offset = drag.anchor_offset;
        let mut direct_resize = None;
        self.snap_feedback = SnapResult::default();
        let snapping = (!modifiers.contains(shrimply_preview_provider_skia::Modifiers::CONTROL))
            .then_some(self.snap_scene.as_ref())
            .flatten();
        match drag.kind {
            DragKind::Move => {
                next.position += delta;
                if let Some(snapping) = snapping {
                    let snap = snapping.snap_geometry(next.matrix(), drag.bounds);
                    next.position += snap.delta;
                    self.snap_feedback = snap;
                }
            }
            DragKind::Anchor => {
                if let Some(snapping) = snapping {
                    let snap = snapping.snap_point_to_geometry(current_canvas, self.geometry);
                    current_canvas += snap.delta;
                    self.snap_feedback = snap;
                }
                let linear = linear_transform(drag.start);
                let determinant = linear.determinant();
                if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                    return false;
                }
                next.position = current_canvas;
                next.anchor += linear
                    .inverse()
                    .transform_vector2(current_canvas - drag.start.position);
            }
            DragKind::Rotate => {
                let anchor = viewport
                    .canvas_to_screen
                    .transform_point2(drag.start.position);
                let start_angle = (drag.start_pointer - anchor)
                    .y
                    .atan2((drag.start_pointer - anchor).x);
                let angle = (point - anchor).y.atan2((point - anchor).x);
                next.rotation_degrees += (angle - start_angle).to_degrees();
                if let Some(snapping) = snapping {
                    let radius_px = (point - anchor).length();
                    if let Some(rotation) = snapping.snap_angle(
                        next.rotation_degrees,
                        radius_px,
                        ROTATION_SNAP_STEP_DEGREES,
                    ) {
                        next.rotation_degrees = rotation;
                    }
                }
            }
            DragKind::Resize(handle) => {
                let axes = resize_axes(handle);
                let fixed_local = if drag.centered_resize {
                    drag.bounds.center()
                } else {
                    bounds_handle_position(opposite_handle(handle), drag.bounds)
                };
                let handle_local = bounds_handle_position(handle, drag.bounds);
                let fixed_canvas = drag.start.matrix().transform_point2(fixed_local);
                let unscaled = unscaled_transform(drag.start);
                let determinant = unscaled.determinant();
                if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
                    return false;
                }
                let target_local = unscaled
                    .inverse()
                    .transform_vector2(current_canvas - fixed_canvas);
                let local_span = handle_local - fixed_local;
                let mut proposed = drag.start.scale.max(Vec2::splat(MIN_SCALE));
                if axes.x != 0.0 {
                    proposed.x = (target_local.x / local_span.x).max(MIN_SCALE);
                }
                if axes.y != 0.0 {
                    proposed.y = (target_local.y / local_span.y).max(MIN_SCALE);
                }
                let mut factor = uniform_factor(drag.start.scale, proposed, axes);
                if drag.free_resize || matches!(self.resize, ResizeTarget::Transform) {
                    let mut scale = if drag.free_resize {
                        Vec2::new(
                            if axes.x == 0.0 {
                                drag.start.scale.x
                            } else {
                                proposed.x
                            },
                            if axes.y == 0.0 {
                                drag.start.scale.y
                            } else {
                                proposed.y
                            },
                        )
                    } else {
                        drag.start.scale * factor
                    }
                    .max(Vec2::splat(MIN_SCALE));
                    set_resize_transform(&mut next, scale, fixed_local, fixed_canvas);
                    if let Some(snapping) = snapping {
                        let rect = transformed_rect(next.matrix(), drag.bounds);
                        let (x_features, y_features) = if drag.free_resize {
                            let mut probes = [None; 2];
                            for index in 0..2 {
                                if axes[index] == 0.0 {
                                    continue;
                                }
                                let mut probe_scale = scale;
                                probe_scale[index] +=
                                    scale[index].abs().max(1.0) * SNAP_SOLVER_STEP;
                                let mut probe = next;
                                set_resize_transform(
                                    &mut probe,
                                    probe_scale,
                                    fixed_local,
                                    fixed_canvas,
                                );
                                probes[index] = Some(transformed_rect(probe.matrix(), drag.bounds));
                            }
                            moving_rect_features(rect, probes)
                        } else {
                            rect_features_away_from_point(rect, fixed_canvas)
                        };
                        let constraints =
                            snapping.snap_rect_with_features(rect, x_features, y_features);
                        if constraints.x.is_some() || constraints.y.is_some() {
                            if drag.free_resize {
                                let solved = solve_free_scale_snap(
                                    next,
                                    scale,
                                    axes,
                                    fixed_local,
                                    fixed_canvas,
                                    drag.bounds,
                                    constraints,
                                );
                                scale = solved.0;
                                set_resize_transform(&mut next, scale, fixed_local, fixed_canvas);
                                self.snap_feedback = resolved_snap(
                                    solved.1,
                                    transformed_rect(next.matrix(), drag.bounds),
                                );
                            } else {
                                let solved = solve_uniform_scale_snap(
                                    next,
                                    factor,
                                    drag.start.scale,
                                    fixed_local,
                                    fixed_canvas,
                                    drag.bounds,
                                    constraints,
                                );
                                factor = solved.0;
                                scale = (drag.start.scale * factor).max(Vec2::splat(MIN_SCALE));
                                set_resize_transform(&mut next, scale, fixed_local, fixed_canvas);
                                self.snap_feedback = resolved_snap(
                                    solved.1,
                                    transformed_rect(next.matrix(), drag.bounds),
                                );
                            }
                        }
                    }
                    next.position = fixed_canvas
                        - linear_transform(next).transform_vector2(fixed_local - next.anchor);
                } else {
                    direct_resize = Some((handle, fixed_canvas, factor));
                }
            }
        }

        let mut changed = false;
        if let Some((handle, fixed_canvas, mut factor)) = direct_resize {
            let mut constraints = None;
            let mut previous_sample = None;
            for snap_pass in 0..DIRECT_RESIZE_SNAP_PASSES {
                let time = edits.keyframe_time();
                let item = edits
                    .target_mut(self.target)
                    .downcast_mut::<VisualItem>()
                    .expect("visual preview target has the wrong type");
                changed |= match (self.resize, &mut item.content) {
                    (ResizeTarget::Text { font_size, .. }, VisualSource::Text(text)) => {
                        set_scalar(&mut text.font_size, time, (font_size * factor).max(1.0))
                    }
                    (ResizeTarget::Shape { size, .. }, VisualSource::Shape(shape)) => {
                        set_vec2(&mut shape.size, time, (size * factor).max(Vec2::ONE))
                    }
                    _ => unreachable!(),
                };
                let resized = edits
                    .updated_geometry(self.target)
                    .expect("resized visual item has no 2D geometry");
                next_bounds = resized.bounds;
                next_source_size = resized.source_size;
                next_anchor_offset = resized.anchor_offset;
                next.anchor = (drag.start.anchor - drag.anchor_offset) / drag.source_size
                    * next_source_size
                    + next_anchor_offset;
                let next_fixed = if drag.centered_resize {
                    next_bounds.center()
                } else {
                    bounds_handle_position(opposite_handle(handle), next_bounds)
                };
                next.position = fixed_canvas
                    - linear_transform(next).transform_vector2(next_fixed - next.anchor);
                let Some(snapping) = snapping else {
                    break;
                };
                let rect = transformed_rect(next.matrix(), next_bounds);
                let (x_features, y_features) = rect_features_away_from_point(rect, fixed_canvas);
                let mut selected = *constraints.get_or_insert_with(|| {
                    snapping.snap_rect_with_features(rect, x_features, y_features)
                });
                if selected.x.is_none() && selected.y.is_none() {
                    break;
                }
                let mut sources = snap_sources(selected, rect);
                let mut residual = snap_residual(selected, sources);
                if residual.x.abs() <= SNAP_RESIDUAL_TOLERANCE
                    && residual.y.abs() <= SNAP_RESIDUAL_TOLERANCE
                {
                    self.snap_feedback = resolved_snap(selected, rect);
                    break;
                }
                let mut derivative = previous_sample
                    .filter(|(previous_factor, _): &(f32, Vec2)| {
                        (factor - previous_factor).abs() > f32::EPSILON
                    })
                    .map(|(previous_factor, previous_sources)| {
                        (sources - previous_sources) / (factor - previous_factor)
                    })
                    .unwrap_or_else(|| {
                        Vec2::new(
                            selected
                                .x
                                .map_or(0.0, |_| (sources.x - fixed_canvas.x) / factor),
                            selected
                                .y
                                .map_or(0.0, |_| (sources.y - fixed_canvas.y) / factor),
                        )
                    });
                selected = attainable_scalar_constraints(selected, derivative, residual);
                constraints = Some(selected);
                sources = snap_sources(selected, rect);
                residual = snap_residual(selected, sources);
                if selected.x.is_none() {
                    derivative.x = 0.0;
                }
                if selected.y.is_none() {
                    derivative.y = 0.0;
                }
                self.snap_feedback = resolved_snap(selected, rect);
                if selected.x.is_none() && selected.y.is_none()
                    || snap_pass + 1 == DIRECT_RESIZE_SNAP_PASSES
                {
                    break;
                }
                let denominator = derivative.length_squared();
                if !denominator.is_finite() || denominator <= f32::EPSILON {
                    self.snap_feedback = SnapResult::default();
                    break;
                }
                previous_sample = Some((factor, sources));
                let corrected = factor + residual.dot(derivative) / denominator;
                if !corrected.is_finite() {
                    break;
                }
                factor = corrected.max(MIN_SCALE);
            }
        }
        let time = edits.keyframe_time();
        let item = edits
            .target_mut(self.target)
            .downcast_mut::<VisualItem>()
            .expect("visual preview target has the wrong type");
        changed |= set_vec2(&mut item.transform.position, time, next.position)
            | set_vec2(&mut item.transform.anchor, time, next.anchor)
            | set_vec2(&mut item.transform.scale, time, next.scale)
            | set_scalar(
                &mut item.transform.rotation_degrees,
                time,
                next.rotation_degrees,
            );
        if changed {
            self.geometry.transform = next;
            self.geometry.local_to_canvas = next.matrix();
            self.geometry.bounds = next_bounds;
            self.geometry.source_size = next_source_size;
            self.geometry.anchor_offset = next_anchor_offset;
            if let Some(drag) = &mut self.drag {
                drag.changed = true;
            }
        }
        changed
    }
}

impl PreviewProvider for TransformHandler {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = context.selection_color();
        let viewport = context.viewport();
        for (axis, snap) in [
            (SnapAxis::X, self.snap_feedback.x),
            (SnapAxis::Y, self.snap_feedback.y),
        ] {
            let Some(snap) = snap else { continue };
            match snap.kind {
                AxisSnapKind::Align => {
                    draw_snap_axis_line(painter, viewport, axis, snap.target, color);
                }
                AxisSnapKind::EqualGap {
                    first,
                    second,
                    cross,
                } => {
                    draw_snap_gap(painter, viewport, axis, first, cross, color);
                    draw_snap_gap(painter, viewport, axis, second, cross, color);
                }
                AxisSnapKind::Mirror {
                    center,
                    peer,
                    cross,
                } => {
                    draw_snap_axis_line(painter, viewport, axis, center, color);
                    draw_snap_gap(
                        painter,
                        viewport,
                        axis,
                        AxisGap {
                            start: peer,
                            end: center,
                        },
                        cross,
                        color,
                    );
                    draw_snap_gap(
                        painter,
                        viewport,
                        axis,
                        AxisGap {
                            start: center,
                            end: snap.target,
                        },
                        cross,
                        color,
                    );
                }
            }
        }
        let map = self.screen_map(context);
        draw_control_rect(painter, map, self.geometry.bounds, color);
        if self.position_editable
            && (self.scale_editable
                || matches!(
                    self.resize,
                    ResizeTarget::Text { editable: true, .. }
                        | ResizeTarget::Shape { editable: true, .. }
                ))
        {
            let handles = BOUNDS_HANDLES.map(|handle| {
                map.transform_point2(bounds_handle_position(handle, self.geometry.bounds))
            });
            draw_keypoints(painter, &handles, color);
        }
        if self.rotation_editable {
            let top = map.transform_point2(Vec2::new(
                self.geometry.bounds.center().x,
                self.geometry.bounds.min.y,
            ));
            let rotation = self.rotation_handle(context);
            draw_control_line(painter, top, rotation, color);
            draw_keypoint(painter, rotation, color);
        }
        let anchor = self.anchor(context);
        shrimply_preview_provider_skia::drawing::circle(
            painter,
            anchor,
            7.0,
            shrimply_preview_provider_skia::Paint::stroke(
                shrimply_preview_provider_skia::Stroke::new(color, 2.0),
            ),
        );
        draw_control_line(
            painter,
            anchor - Vec2::X * 11.0,
            anchor - Vec2::X * 7.0,
            color,
        );
        draw_control_line(
            painter,
            anchor + Vec2::X * 7.0,
            anchor + Vec2::X * 11.0,
            color,
        );
        draw_control_line(
            painter,
            anchor - Vec2::Y * 11.0,
            anchor - Vec2::Y * 7.0,
            color,
        );
        draw_control_line(
            painter,
            anchor + Vec2::Y * 7.0,
            anchor + Vec2::Y * 11.0,
            color,
        );
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        match event {
            shrimply_preview_provider_skia::PointerEvent::Hover(input) => PreviewResponse {
                handled: self
                    .hit(input.sample.position, input.modifiers, context)
                    .is_some(),
                redraw: false,
                cursor: self
                    .hit(input.sample.position, input.modifiers, context)
                    .map_or(CursorUpdate::Clear, |(_, cursor)| CursorUpdate::Set(cursor)),
                edit: PreviewEditOutcome::UNCHANGED,
            },
            shrimply_preview_provider_skia::PointerEvent::Leave => PreviewResponse {
                cursor: CursorUpdate::Clear,
                ..PreviewResponse::IGNORED
            },
            shrimply_preview_provider_skia::PointerEvent::Begin(input) => {
                let Some((kind, cursor)) =
                    self.hit(input.sample.position, input.modifiers, context)
                else {
                    return PreviewResponse::IGNORED;
                };
                self.snapshot = Some(
                    edits
                        .target_mut(self.target)
                        .downcast_mut::<VisualItem>()
                        .expect("visual preview target has the wrong type")
                        .clone(),
                );
                self.drag = Some(DragState {
                    kind,
                    free_resize: input
                        .modifiers
                        .contains(shrimply_preview_provider_skia::Modifiers::CONTROL),
                    centered_resize: input
                        .modifiers
                        .contains(shrimply_preview_provider_skia::Modifiers::ALT),
                    start_pointer: input.sample.position,
                    start: self.geometry.transform,
                    bounds: self.geometry.bounds,
                    source_size: self.geometry.source_size,
                    anchor_offset: self.geometry.anchor_offset,
                    changed: false,
                });
                self.snap_scene = context.snapping().cloned();
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
                let changed = self.apply_drag(point, input.modifiers, context, edits);
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::live(PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR)
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            shrimply_preview_provider_skia::PointerEvent::End(_) => {
                let changed = self.drag.take().is_some_and(|drag| drag.changed);
                self.snapshot = None;
                self.snap_scene = None;
                self.snap_feedback = SnapResult::default();
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
                self.snap_scene = None;
                self.snap_feedback = SnapResult::default();
                if changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<VisualItem>()
                        .expect("visual preview target has the wrong type") = self
                        .snapshot
                        .take()
                        .expect("changed visual drag has a snapshot");
                }
                self.snapshot = None;
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

fn resize_axes(handle: BoundsHandle) -> Vec2 {
    match handle {
        BoundsHandle::TopLeft => Vec2::new(-1.0, -1.0),
        BoundsHandle::Top => Vec2::new(0.0, -1.0),
        BoundsHandle::TopRight => Vec2::new(1.0, -1.0),
        BoundsHandle::Right => Vec2::new(1.0, 0.0),
        BoundsHandle::BottomRight => Vec2::new(1.0, 1.0),
        BoundsHandle::Bottom => Vec2::new(0.0, 1.0),
        BoundsHandle::BottomLeft => Vec2::new(-1.0, 1.0),
        BoundsHandle::Left => Vec2::new(-1.0, 0.0),
    }
}

fn opposite_handle(handle: BoundsHandle) -> BoundsHandle {
    match handle {
        BoundsHandle::TopLeft => BoundsHandle::BottomRight,
        BoundsHandle::Top => BoundsHandle::Bottom,
        BoundsHandle::TopRight => BoundsHandle::BottomLeft,
        BoundsHandle::Right => BoundsHandle::Left,
        BoundsHandle::BottomRight => BoundsHandle::TopLeft,
        BoundsHandle::Bottom => BoundsHandle::Top,
        BoundsHandle::BottomLeft => BoundsHandle::TopRight,
        BoundsHandle::Left => BoundsHandle::Right,
    }
}

fn rect_features_away_from_point(rect: Rect, fixed: Vec2) -> (AxisFeatures, AxisFeatures) {
    (
        axis_features_away(rect.min.x, rect.center().x, rect.max.x, fixed.x),
        axis_features_away(rect.min.y, rect.center().y, rect.max.y, fixed.y),
    )
}

fn axis_features_away(minimum: f32, center: f32, maximum: f32, fixed: f32) -> AxisFeatures {
    let tolerance = feature_motion_tolerance([minimum, center, maximum, fixed]);
    let mut features = AxisFeatures::NONE;
    for (feature, value) in [
        (AxisFeatures::MINIMUM, minimum),
        (AxisFeatures::CENTER, center),
        (AxisFeatures::MAXIMUM, maximum),
    ] {
        if (value - fixed).abs() > tolerance {
            features = features | feature;
        }
    }
    features
}

fn moving_rect_features(current: Rect, probes: [Option<Rect>; 2]) -> (AxisFeatures, AxisFeatures) {
    let mut features = [AxisFeatures::NONE; 2];
    for probe in probes.into_iter().flatten() {
        for (axis, current, probe) in [
            (
                0,
                [current.min.x, current.center().x, current.max.x],
                [probe.min.x, probe.center().x, probe.max.x],
            ),
            (
                1,
                [current.min.y, current.center().y, current.max.y],
                [probe.min.y, probe.center().y, probe.max.y],
            ),
        ] {
            let tolerance = feature_motion_tolerance([
                current[0], current[1], current[2], probe[0], probe[1], probe[2],
            ]);
            for (feature, index) in [
                (AxisFeatures::MINIMUM, 0),
                (AxisFeatures::CENTER, 1),
                (AxisFeatures::MAXIMUM, 2),
            ] {
                if (probe[index] - current[index]).abs() > tolerance {
                    features[axis] = features[axis] | feature;
                }
            }
        }
    }
    (features[0], features[1])
}

fn feature_motion_tolerance<const N: usize>(values: [f32; N]) -> f32 {
    values.into_iter().map(f32::abs).fold(1.0, f32::max) * f32::EPSILON * 64.0
}

fn shear_transform(shear: Vec2) -> Mat3 {
    Mat3::from_cols_array(&[1.0, shear.y, 0.0, shear.x, 1.0, 0.0, 0.0, 0.0, 1.0])
}

fn unscaled_transform(transform: ResolvedTransform) -> Mat3 {
    Mat3::from_angle(transform.rotation_degrees.to_radians()) * shear_transform(transform.shear)
}

fn linear_transform(transform: ResolvedTransform) -> Mat3 {
    unscaled_transform(transform) * Mat3::from_scale(transform.scale)
}

fn uniform_factor(start: Vec2, proposed: Vec2, axes: Vec2) -> f32 {
    let mut factor = 1.0;
    let mut distance = -1.0;
    for (affected, start, proposed) in [
        (axes.x != 0.0, start.x, proposed.x),
        (axes.y != 0.0, start.y, proposed.y),
    ] {
        if !affected || start.abs() <= f32::EPSILON || !proposed.is_finite() {
            continue;
        }
        let candidate = proposed / start;
        let candidate_distance = (candidate - 1.0).abs();
        if candidate_distance > distance {
            factor = candidate;
            distance = candidate_distance;
        }
    }
    factor.max(MIN_SCALE)
}

fn set_resize_transform(
    transform: &mut ResolvedTransform,
    scale: Vec2,
    fixed_local: Vec2,
    fixed_canvas: Vec2,
) {
    transform.scale = scale.max(Vec2::splat(MIN_SCALE));
    transform.position = fixed_canvas
        - linear_transform(*transform).transform_vector2(fixed_local - transform.anchor);
}

fn solve_uniform_scale_snap(
    mut transform: ResolvedTransform,
    mut factor: f32,
    start_scale: Vec2,
    fixed_local: Vec2,
    fixed_canvas: Vec2,
    bounds: Rect,
    mut constraints: SnapResult,
) -> (f32, SnapResult) {
    for _ in 0..DIRECT_RESIZE_SNAP_PASSES {
        let current = snap_sources(constraints, transformed_rect(transform.matrix(), bounds));
        let mut residual = snap_residual(constraints, current);
        if residual.length_squared() <= f32::EPSILON {
            break;
        }
        let step = factor.abs().max(1.0) * SNAP_SOLVER_STEP;
        let mut probe = transform;
        set_resize_transform(
            &mut probe,
            start_scale * (factor + step),
            fixed_local,
            fixed_canvas,
        );
        let mut derivative =
            (snap_sources(constraints, transformed_rect(probe.matrix(), bounds)) - current) / step;
        constraints = attainable_scalar_constraints(constraints, derivative, residual);
        if constraints.x.is_none() {
            derivative.x = 0.0;
            residual.x = 0.0;
        }
        if constraints.y.is_none() {
            derivative.y = 0.0;
            residual.y = 0.0;
        }
        let denominator = derivative.length_squared();
        if !denominator.is_finite() || denominator <= f32::EPSILON {
            constraints = SnapResult::default();
            break;
        }
        factor = (factor + residual.dot(derivative) / denominator).max(MIN_SCALE);
        if !factor.is_finite() {
            break;
        }
        set_resize_transform(
            &mut transform,
            start_scale * factor,
            fixed_local,
            fixed_canvas,
        );
    }
    let feedback = satisfied_snap(constraints, transformed_rect(transform.matrix(), bounds));
    (factor, feedback)
}

fn solve_free_scale_snap(
    mut transform: ResolvedTransform,
    mut scale: Vec2,
    axes: Vec2,
    fixed_local: Vec2,
    fixed_canvas: Vec2,
    bounds: Rect,
    mut constraints: SnapResult,
) -> (Vec2, SnapResult) {
    for _ in 0..DIRECT_RESIZE_SNAP_PASSES {
        let current = snap_sources(constraints, transformed_rect(transform.matrix(), bounds));
        let mut residual = snap_residual(constraints, current);
        if residual.length_squared() <= f32::EPSILON {
            break;
        }
        let mut derivatives = [Vec2::ZERO; 2];
        for index in 0..2 {
            if axes[index] == 0.0 {
                continue;
            }
            let step = scale[index].abs().max(1.0) * SNAP_SOLVER_STEP;
            let mut probe_scale = scale;
            probe_scale[index] += step;
            let mut probe = transform;
            set_resize_transform(&mut probe, probe_scale, fixed_local, fixed_canvas);
            derivatives[index] =
                (snap_sources(constraints, transformed_rect(probe.matrix(), bounds)) - current)
                    / step;
        }
        if (axes.x == 0.0) != (axes.y == 0.0) {
            let derivative = if axes.x == 0.0 {
                derivatives[1]
            } else {
                derivatives[0]
            };
            constraints = attainable_scalar_constraints(constraints, derivative, residual);
            if constraints.x.is_none() {
                derivatives[0].x = 0.0;
                derivatives[1].x = 0.0;
                residual.x = 0.0;
            }
            if constraints.y.is_none() {
                derivatives[0].y = 0.0;
                derivatives[1].y = 0.0;
                residual.y = 0.0;
            }
        }
        let correction =
            solve_parameter_correction(derivatives, residual, [axes.x != 0.0, axes.y != 0.0]);
        if !correction.is_finite() || correction.length_squared() <= f32::EPSILON {
            break;
        }
        for index in 0..2 {
            if axes[index] != 0.0 {
                scale[index] = (scale[index] + correction[index]).max(MIN_SCALE);
            }
        }
        set_resize_transform(&mut transform, scale, fixed_local, fixed_canvas);
    }
    let feedback = satisfied_snap(constraints, transformed_rect(transform.matrix(), bounds));
    (scale, feedback)
}

fn snap_sources(constraints: SnapResult, rect: Rect) -> Vec2 {
    Vec2::new(
        constraints.x.map_or(0.0, |snap| {
            rect_axis_feature(rect, SnapAxis::X, snap.feature)
        }),
        constraints.y.map_or(0.0, |snap| {
            rect_axis_feature(rect, SnapAxis::Y, snap.feature)
        }),
    )
}

fn snap_residual(constraints: SnapResult, sources: Vec2) -> Vec2 {
    Vec2::new(
        constraints.x.map_or(0.0, |snap| snap.target - sources.x),
        constraints.y.map_or(0.0, |snap| snap.target - sources.y),
    )
}

fn attainable_scalar_constraints(
    mut constraints: SnapResult,
    derivative: Vec2,
    residual: Vec2,
) -> SnapResult {
    if !derivative.x.is_finite() || derivative.x.abs() <= f32::EPSILON {
        constraints.x = None;
    }
    if !derivative.y.is_finite() || derivative.y.abs() <= f32::EPSILON {
        constraints.y = None;
    }
    let (Some(x), Some(y)) = (constraints.x, constraints.y) else {
        return constraints;
    };

    let denominator = derivative.length_squared();
    let correction = residual.dot(derivative) / denominator;
    let remaining = residual - derivative * correction;
    if remaining.x.abs() <= SNAP_RESIDUAL_TOLERANCE && remaining.y.abs() <= SNAP_RESIDUAL_TOLERANCE
    {
        return constraints;
    }

    let x_correction = residual.x / derivative.x;
    let y_correction = residual.y / derivative.y;
    let choose_x = match x_correction.abs().total_cmp(&y_correction.abs()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Equal => x.priority <= y.priority,
        std::cmp::Ordering::Greater => false,
    };
    if choose_x {
        constraints.y = None;
    } else {
        constraints.x = None;
    }
    constraints
}

fn satisfied_snap(mut constraints: SnapResult, rect: Rect) -> SnapResult {
    let residual = snap_residual(constraints, snap_sources(constraints, rect));
    if !residual.x.is_finite() || residual.x.abs() > SNAP_RESIDUAL_TOLERANCE {
        constraints.x = None;
    }
    if !residual.y.is_finite() || residual.y.abs() > SNAP_RESIDUAL_TOLERANCE {
        constraints.y = None;
    }
    constraints
}

fn resolved_snap(constraints: SnapResult, rect: Rect) -> SnapResult {
    let constraints = satisfied_snap(constraints, rect);
    let x = constraints
        .x
        .map(|snap| resolve_axis_snap(snap, rect, SnapAxis::X));
    let y = constraints
        .y
        .map(|snap| resolve_axis_snap(snap, rect, SnapAxis::Y));
    SnapResult {
        delta: Vec2::new(
            x.map_or(0.0, |snap| snap.delta),
            y.map_or(0.0, |snap| snap.delta),
        ),
        x,
        y,
    }
}

fn transformed_resize_cursor(handle: BoundsHandle, map: Mat3) -> Cursor {
    let direction = map.transform_vector2(resize_axes(handle));
    if !direction.is_finite() || direction.length_squared() <= f32::EPSILON {
        return Cursor::Move;
    }
    match ((direction.y.atan2(direction.x).to_degrees() / 45.0).round() as i32).rem_euclid(4) {
        0 => Cursor::ResizeHorizontal,
        1 => Cursor::ResizeDiagonalDown,
        2 => Cursor::ResizeVertical,
        _ => Cursor::ResizeDiagonalUp,
    }
}

fn draw_snap_axis_line(
    painter: &shrimply_preview_provider_skia::PreviewCanvas,
    viewport: PreviewViewport,
    axis: SnapAxis,
    value: f32,
    color: Color,
) {
    let (start, end) = match axis {
        SnapAxis::X => (
            Vec2::new(value, 0.0),
            Vec2::new(value, viewport.canvas_size.y),
        ),
        SnapAxis::Y => (
            Vec2::new(0.0, value),
            Vec2::new(viewport.canvas_size.x, value),
        ),
    };
    draw_control_line(
        painter,
        viewport.canvas_point_to_screen(start),
        viewport.canvas_point_to_screen(end),
        color,
    );
}

fn draw_snap_gap(
    painter: &shrimply_preview_provider_skia::PreviewCanvas,
    viewport: PreviewViewport,
    axis: SnapAxis,
    gap: AxisGap,
    cross: f32,
    color: Color,
) {
    let (start, end, cap) = match axis {
        SnapAxis::X => (
            Vec2::new(gap.start, cross),
            Vec2::new(gap.end, cross),
            Vec2::Y * SNAP_MARK_CAP_HALF_LENGTH,
        ),
        SnapAxis::Y => (
            Vec2::new(cross, gap.start),
            Vec2::new(cross, gap.end),
            Vec2::X * SNAP_MARK_CAP_HALF_LENGTH,
        ),
    };
    let start = viewport.canvas_point_to_screen(start);
    let end = viewport.canvas_point_to_screen(end);
    draw_control_line(painter, start, end, color);
    draw_control_line(painter, start - cap, start + cap, color);
    draw_control_line(painter, end - cap, end + cap, color);
}

pub(super) fn set_scalar(value: &mut TimelineValue<f32>, time: Time, next: f32) -> bool {
    editable(value)
        && next.is_finite()
        && set_curve(value, time, next, |left, right| {
            (left - right).abs() <= f32::EPSILON
        })
}

pub(super) fn set_vec2(value: &mut TimelineValue<Vec2>, time: Time, next: Vec2) -> bool {
    editable(value)
        && next.is_finite()
        && set_curve(value, time, next, |left, right| {
            left.distance_squared(*right) <= f32::EPSILON
        })
}

pub(super) fn set_vec3(value: &mut TimelineValue<Vec3>, time: Time, next: Vec3) -> bool {
    editable(value)
        && next.is_finite()
        && set_curve(value, time, next, |left, right| {
            left.distance_squared(*right) <= f32::EPSILON
        })
}

pub(super) fn editable<T: TimelineValueType>(value: &TimelineValue<T>) -> bool {
    !value
        .expression
        .as_ref()
        .is_some_and(|expression| expression.enabled)
}

fn set_curve<T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>>(
    value: &mut TimelineValue<T>,
    time: Time,
    next: T,
    equal: impl Fn(&T, &T) -> bool,
) -> bool {
    match &mut value.base {
        TimelineBase::Const(current) => {
            if equal(current, &next) {
                return false;
            }
            *current = next;
        }
        TimelineBase::Keyframes(keyframes) => {
            if let Some(keyframe) = keyframes
                .iter_mut()
                .find(|keyframe| keyframe.time.approx_eq(time))
            {
                if keyframe.time == time && equal(&keyframe.value, &next) {
                    return false;
                }
                keyframe.time = time;
                keyframe.value = next;
                keyframes.sort_by_key(|keyframe| keyframe.time);
            } else {
                keyframes.push(TimelineCurveKeyframe {
                    id: Uuid::new_v4(),
                    time,
                    value: next,
                    interpolation_to_next: Default::default(),
                });
                keyframes.sort_by_key(|keyframe| keyframe.time);
            }
        }
    }
    true
}
