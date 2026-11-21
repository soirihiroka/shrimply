use glam::{Mat2, Mat3, Vec2};
use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, Paint, PointerEvent, PreviewBuilder, PreviewContext,
    PreviewEditOutcome, PreviewEditSink, PreviewFacetKey, PreviewItemGeometry, PreviewProvider,
    PreviewRefresh, PreviewResponse, PreviewTarget, Stroke, draw_control_line, draw_control_rect,
    draw_keypoint, hit_keypoint,
};

use super::super::{ShapeItem, ShapeKind};

pub const CONTENT_FACET: PreviewFacetKey = PreviewFacetKey::new("shape.content");
pub const APPEARANCE_FACET: PreviewFacetKey = PreviewFacetKey::new("shape.appearance");

const MINIMUM_PERCENT: f32 = 5.0;
const MAXIMUM_PERCENT: f32 = 95.0;
const FULL_ELLIPSE_DEGREES: f32 = 360.0;
const GUIDE_WIDTH: f32 = 1.5;
const ELLIPSE_SEGMENTS: usize = 48;

pub fn provider(
    shape: &ShapeItem,
    target: PreviewTarget,
    geometry: PreviewItemGeometry,
    builder: &impl PreviewBuilder,
) -> Box<dyn PreviewProvider> {
    assert!(matches!(target.facet(), CONTENT_FACET | APPEARANCE_FACET));
    Box::new(ShapeHandler {
        target,
        geometry,
        controls: Controls::new(shape, geometry, builder),
        drag: None,
    })
}

#[derive(Clone, Copy)]
struct Controls {
    shape: ShapeKind,
    star_points: u32,
    star_inner_radius_percent: f32,
    star_inner_editable: bool,
    arrow_shaft_width_percent: f32,
    arrow_shaft_editable: bool,
    arrow_head_length_percent: f32,
    arrow_head_editable: bool,
    cross_arm_thickness_percent: f32,
    cross_arm_editable: bool,
    ellipse_inner_radius_percent: f32,
    ellipse_inner_editable: bool,
    ellipse_completion_degrees: f32,
    outline_width: f32,
    corner_radius: f32,
    corner_editable: bool,
    shadow_distance: f32,
    shadow_distance_editable: bool,
    shadow_direction_degrees: f32,
    shadow_direction_editable: bool,
    shadow_width: f32,
    shadow_blur: f32,
}

impl Controls {
    fn new(
        shape: &ShapeItem,
        geometry: PreviewItemGeometry,
        builder: &impl PreviewBuilder,
    ) -> Self {
        Self {
            shape: builder.resolve(&shape.shape),
            star_points: builder.resolve(&shape.star_points).clamp(3, 32),
            star_inner_radius_percent: builder
                .resolve(&shape.star_inner_radius_percent)
                .clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT),
            star_inner_editable: super::visual_item::editable(&shape.star_inner_radius_percent),
            arrow_shaft_width_percent: builder
                .resolve(&shape.arrow_shaft_width_percent)
                .clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT),
            arrow_shaft_editable: super::visual_item::editable(&shape.arrow_shaft_width_percent),
            arrow_head_length_percent: builder
                .resolve(&shape.arrow_head_length_percent)
                .clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT),
            arrow_head_editable: super::visual_item::editable(&shape.arrow_head_length_percent),
            cross_arm_thickness_percent: builder
                .resolve(&shape.cross_arm_thickness_percent)
                .clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT),
            cross_arm_editable: super::visual_item::editable(&shape.cross_arm_thickness_percent),
            ellipse_inner_radius_percent: builder
                .resolve(&shape.ellipse_inner_radius_percent)
                .clamp(0.0, MAXIMUM_PERCENT),
            ellipse_inner_editable: super::visual_item::editable(
                &shape.ellipse_inner_radius_percent,
            ),
            ellipse_completion_degrees: builder
                .resolve(&shape.ellipse_completion_degrees)
                .clamp(0.0, FULL_ELLIPSE_DEGREES),
            outline_width: builder.resolve(&shape.outline_width).max(0.0),
            corner_radius: builder.resolve(&shape.corner_radius).max(0.0),
            corner_editable: super::visual_item::editable(&shape.corner_radius),
            shadow_distance: builder.resolve(&shape.shadow_distance).max(0.0),
            shadow_distance_editable: super::visual_item::editable(&shape.shadow_distance),
            shadow_direction_degrees: builder.resolve(&shape.shadow_direction_degrees),
            shadow_direction_editable: super::visual_item::editable(
                &shape.shadow_direction_degrees,
            ),
            shadow_width: builder.resolve(&shape.shadow_width).max(0.0),
            shadow_blur: builder.resolve(&shape.shadow_blur).max(0.0),
        }
        .clamp_corner(geometry.bounds.size())
    }

    fn clamp_corner(mut self, size: Vec2) -> Self {
        self.corner_radius = self.corner_radius.min(size.min_element().max(0.0) * 0.5);
        self
    }

    fn shadow_offset(self, rotation_degrees: f32) -> Vec2 {
        Mat2::from_angle(-rotation_degrees.to_radians())
            * Vec2::from_angle(self.shadow_direction_degrees.to_radians())
            * self.shadow_distance
    }
}

#[derive(Clone, Copy)]
enum Control {
    StarInnerRadius,
    ArrowShaftWidth,
    ArrowHeadLength,
    CrossArmThickness,
    EllipseInnerRadius,
    CornerRadius,
    Shadow,
}

struct DragState {
    control: Control,
    changed: bool,
    snapshot: ShapeItem,
    controls: Controls,
}

struct ShapeHandler {
    target: PreviewTarget,
    geometry: PreviewItemGeometry,
    controls: Controls,
    drag: Option<DragState>,
}

impl ShapeHandler {
    fn screen_map(&self, context: &dyn PreviewContext) -> Mat3 {
        context.viewport().canvas_to_screen * self.geometry.local_to_canvas
    }

    fn local_to_screen(&self, point: Vec2, context: &dyn PreviewContext) -> Vec2 {
        self.screen_map(context).transform_point2(point)
    }

    fn screen_to_local(&self, point: Vec2, context: &dyn PreviewContext) -> Option<Vec2> {
        let map = self.screen_map(context);
        let determinant = map.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return None;
        }
        let point = map.inverse().transform_point2(point);
        point.is_finite().then_some(point)
    }

    fn origin(&self) -> Vec2 {
        self.geometry.bounds.min
    }

    fn size(&self) -> Vec2 {
        self.geometry.bounds.size().max(Vec2::ONE)
    }

    fn center(&self) -> Vec2 {
        self.origin() + self.size() * 0.5
    }

    fn hit(&self, point: Vec2, context: &dyn PreviewContext) -> Option<Control> {
        let hit = |local| hit_keypoint(point, self.local_to_screen(local, context));
        if self.target.facet() == CONTENT_FACET {
            return match self.controls.shape {
                ShapeKind::Star if self.controls.star_inner_editable => hit(self.center()
                    + Vec2::X
                        * self.size().min_element()
                        * 0.5
                        * self.controls.star_inner_radius_percent
                        / 100.0)
                .then_some(Control::StarInnerRadius),
                ShapeKind::Arrow => {
                    let head = Vec2::new(
                        self.origin().x
                            + self.size().x
                                * (1.0 - self.controls.arrow_head_length_percent / 100.0),
                        self.center().y,
                    );
                    let shaft = Vec2::new(
                        self.origin().x + self.size().x * 0.35,
                        self.center().y
                            - self.size().y * self.controls.arrow_shaft_width_percent / 200.0,
                    );
                    if self.controls.arrow_head_editable && hit(head) {
                        Some(Control::ArrowHeadLength)
                    } else {
                        (self.controls.arrow_shaft_editable && hit(shaft))
                            .then_some(Control::ArrowShaftWidth)
                    }
                }
                ShapeKind::Cross if self.controls.cross_arm_editable => hit(Vec2::new(
                    self.center().x
                        + self.size().x * self.controls.cross_arm_thickness_percent / 200.0,
                    self.center().y,
                ))
                .then_some(Control::CrossArmThickness),
                ShapeKind::Ellipse if self.controls.ellipse_inner_editable => self
                    .ellipse_inner_handle()
                    .is_some_and(hit)
                    .then_some(Control::EllipseInnerRadius),
                _ => None,
            };
        }

        let corner = Vec2::new(
            self.origin().x + self.controls.corner_radius,
            self.origin().y,
        );
        if self.controls.corner_editable && hit(corner) {
            return Some(Control::CornerRadius);
        }
        let shadow = self.center()
            + self
                .controls
                .shadow_offset(self.geometry.transform.rotation_degrees);
        ((self.controls.shadow_distance_editable || self.controls.shadow_direction_editable)
            && hit(shadow))
        .then_some(Control::Shadow)
    }

    fn ellipse_geometry(&self) -> Option<EllipseGeometry> {
        let ellipse = shrimply_math_geometry::ellipse_segment(
            self.size(),
            self.controls.ellipse_completion_degrees.max(f32::EPSILON),
        )?;
        Some(EllipseGeometry {
            center: self.origin() + ellipse.center,
            radius: ellipse.radius,
            start_radians: ellipse.start_radians,
            sweep_radians: ellipse.sweep_radians,
        })
    }

    fn ellipse_inner_handle(&self) -> Option<Vec2> {
        let ellipse = self.ellipse_geometry()?;
        let angle = ellipse.start_radians + ellipse.sweep_radians * 0.5;
        Some(
            ellipse.center
                + Vec2::new(angle.cos(), angle.sin())
                    * ellipse.radius
                    * (self.controls.ellipse_inner_radius_percent / 100.0),
        )
    }

    fn apply_drag(
        &mut self,
        point: Vec2,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        let Some(control) = self.drag.as_ref().map(|drag| drag.control) else {
            return false;
        };
        let Some(local) = self.screen_to_local(point, context) else {
            return false;
        };
        let time = edits.keyframe_time();
        let shape = edits
            .target_mut(self.target)
            .downcast_mut::<ShapeItem>()
            .expect("shape preview target has the wrong type");
        let changed = match control {
            Control::StarInnerRadius => {
                let value = (local - self.center()).length()
                    / (self.size().min_element() * 0.5).max(1.0)
                    * 100.0;
                let value = value.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT);
                let changed = super::visual_item::set_scalar(
                    &mut shape.star_inner_radius_percent,
                    time,
                    value,
                );
                self.controls.star_inner_radius_percent = value;
                changed
            }
            Control::ArrowShaftWidth => {
                let value =
                    (local.y - self.center().y).abs() / (self.size().y * 0.5).max(1.0) * 100.0;
                let value = value.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT);
                let changed = super::visual_item::set_scalar(
                    &mut shape.arrow_shaft_width_percent,
                    time,
                    value,
                );
                self.controls.arrow_shaft_width_percent = value;
                changed
            }
            Control::ArrowHeadLength => {
                let value = (1.0 - (local.x - self.origin().x) / self.size().x.max(1.0)) * 100.0;
                let value = value.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT);
                let changed = super::visual_item::set_scalar(
                    &mut shape.arrow_head_length_percent,
                    time,
                    value,
                );
                self.controls.arrow_head_length_percent = value;
                changed
            }
            Control::CrossArmThickness => {
                let value =
                    (local.x - self.center().x).abs() / (self.size().x * 0.5).max(1.0) * 100.0;
                let value = value.clamp(MINIMUM_PERCENT, MAXIMUM_PERCENT);
                let changed = super::visual_item::set_scalar(
                    &mut shape.cross_arm_thickness_percent,
                    time,
                    value,
                );
                self.controls.cross_arm_thickness_percent = value;
                changed
            }
            Control::EllipseInnerRadius => {
                let Some(ellipse) = self.ellipse_geometry() else {
                    return false;
                };
                let normalized = (local - ellipse.center) / ellipse.radius.max(Vec2::ONE);
                let value = (normalized.length() * 100.0).clamp(0.0, MAXIMUM_PERCENT);
                let changed = super::visual_item::set_scalar(
                    &mut shape.ellipse_inner_radius_percent,
                    time,
                    value,
                );
                self.controls.ellipse_inner_radius_percent = value;
                changed
            }
            Control::CornerRadius => {
                let value = (local.x - self.origin().x).clamp(0.0, self.size().min_element() * 0.5);
                let changed = super::visual_item::set_scalar(&mut shape.corner_radius, time, value);
                self.controls.corner_radius = value;
                changed
            }
            Control::Shadow => {
                let offset = local - self.center();
                if offset.length_squared() <= f32::EPSILON {
                    return false;
                }
                let distance = offset.length();
                let direction = offset.y.atan2(offset.x).to_degrees()
                    + self.geometry.transform.rotation_degrees;
                let distance_changed = self.controls.shadow_distance_editable
                    && super::visual_item::set_scalar(&mut shape.shadow_distance, time, distance);
                let direction_changed = self.controls.shadow_direction_editable
                    && super::visual_item::set_scalar(
                        &mut shape.shadow_direction_degrees,
                        time,
                        direction,
                    );
                if self.controls.shadow_distance_editable {
                    self.controls.shadow_distance = distance;
                }
                if self.controls.shadow_direction_editable {
                    self.controls.shadow_direction_degrees = direction;
                }
                distance_changed | direction_changed
            }
        };
        if changed {
            self.drag.as_mut().expect("shape drag disappeared").changed = true;
        }
        changed
    }

    fn draw_content(
        &self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = shrimply_preview_provider_skia::selection_color();
        let guide = Color::new(0.88, 0.88, 0.88, 0.6);
        match self.controls.shape {
            ShapeKind::Star => {
                let outer = self.size().min_element() * 0.5;
                let inner = outer * self.controls.star_inner_radius_percent / 100.0;
                self.draw_local_circle(painter, self.center(), outer, guide, context);
                self.draw_local_circle(painter, self.center(), inner, color, context);
                for index in 0..self.controls.star_points {
                    let angle = -core::f32::consts::FRAC_PI_2
                        + index as f32 * core::f32::consts::TAU / self.controls.star_points as f32;
                    self.draw_local_line(
                        painter,
                        self.center(),
                        self.center() + Vec2::from_angle(angle) * outer,
                        guide,
                        context,
                    );
                }
                draw_keypoint(
                    painter,
                    self.local_to_screen(self.center() + Vec2::X * inner, context),
                    color,
                );
            }
            ShapeKind::Arrow => {
                let head_x = self.origin().x
                    + self.size().x * (1.0 - self.controls.arrow_head_length_percent / 100.0);
                let half_shaft = self.size().y * self.controls.arrow_shaft_width_percent / 200.0;
                for y in [self.center().y - half_shaft, self.center().y + half_shaft] {
                    self.draw_local_line(
                        painter,
                        Vec2::new(self.origin().x, y),
                        Vec2::new(head_x, y),
                        guide,
                        context,
                    );
                }
                self.draw_local_line(
                    painter,
                    Vec2::new(head_x, self.origin().y),
                    Vec2::new(head_x, self.origin().y + self.size().y),
                    color,
                    context,
                );
                draw_keypoint(
                    painter,
                    self.local_to_screen(Vec2::new(head_x, self.center().y), context),
                    color,
                );
                draw_keypoint(
                    painter,
                    self.local_to_screen(
                        Vec2::new(
                            self.origin().x + self.size().x * 0.35,
                            self.center().y - half_shaft,
                        ),
                        context,
                    ),
                    Color::new(1.0, 0.82, 0.24, 1.0),
                );
            }
            ShapeKind::Cross => {
                let half_x = self.size().x * self.controls.cross_arm_thickness_percent / 200.0;
                let half_y = self.size().y * self.controls.cross_arm_thickness_percent / 200.0;
                for (start, end) in [
                    (
                        Vec2::new(self.center().x - half_x, self.origin().y),
                        Vec2::new(self.center().x - half_x, self.origin().y + self.size().y),
                    ),
                    (
                        Vec2::new(self.center().x + half_x, self.origin().y),
                        Vec2::new(self.center().x + half_x, self.origin().y + self.size().y),
                    ),
                    (
                        Vec2::new(self.origin().x, self.center().y - half_y),
                        Vec2::new(self.origin().x + self.size().x, self.center().y - half_y),
                    ),
                    (
                        Vec2::new(self.origin().x, self.center().y + half_y),
                        Vec2::new(self.origin().x + self.size().x, self.center().y + half_y),
                    ),
                ] {
                    self.draw_local_line(painter, start, end, guide, context);
                }
                draw_keypoint(
                    painter,
                    self.local_to_screen(
                        Vec2::new(self.center().x + half_x, self.center().y),
                        context,
                    ),
                    color,
                );
            }
            ShapeKind::Ellipse => {
                if let Some(ellipse) = self.ellipse_geometry() {
                    let points = (0..=ELLIPSE_SEGMENTS)
                        .map(|index| {
                            let progress = index as f32 / ELLIPSE_SEGMENTS as f32;
                            let angle = ellipse.start_radians + ellipse.sweep_radians * progress;
                            self.local_to_screen(
                                ellipse.center
                                    + Vec2::new(angle.cos(), angle.sin()) * ellipse.radius,
                                context,
                            )
                        })
                        .collect::<Vec<_>>();
                    shrimply_preview_provider_skia::drawing::polyline(
                        painter,
                        &points,
                        ellipse.sweep_radians >= core::f32::consts::TAU - f32::EPSILON,
                        Paint::stroke(Stroke::new(guide, GUIDE_WIDTH)),
                    );
                    if let Some(handle) = self.ellipse_inner_handle() {
                        self.draw_local_line(painter, ellipse.center, handle, guide, context);
                        draw_keypoint(painter, self.local_to_screen(handle, context), color);
                    }
                }
            }
            _ => {}
        }
    }

    fn draw_appearance(
        &self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let color = shrimply_preview_provider_skia::selection_color();
        let outline = Vec2::splat(self.controls.outline_width);
        draw_control_rect(
            painter,
            self.screen_map(context),
            shrimply_preview_provider_skia::Rect {
                min: self.origin() - outline,
                max: self.origin() + self.size() + outline,
            },
            color,
        );

        let corner = self.origin() + Vec2::splat(self.controls.corner_radius);
        if self.controls.corner_radius > 0.0 {
            self.draw_local_circle(
                painter,
                corner,
                self.controls.corner_radius,
                Color::new(1.0, 0.82, 0.24, 0.6),
                context,
            );
        }
        draw_keypoint(
            painter,
            self.local_to_screen(
                Vec2::new(
                    self.origin().x + self.controls.corner_radius,
                    self.origin().y,
                ),
                context,
            ),
            Color::new(1.0, 0.82, 0.24, 1.0),
        );

        let shadow = self.center()
            + self
                .controls
                .shadow_offset(self.geometry.transform.rotation_degrees);
        let shadow_color = Color::new(0.86, 0.31, 1.0, 1.0);
        self.draw_local_line(painter, self.center(), shadow, shadow_color, context);
        draw_keypoint(painter, self.local_to_screen(shadow, context), shadow_color);
        let footprint = (self.controls.shadow_width + self.controls.shadow_blur).max(0.0);
        if footprint > 0.0 {
            let footprint = Vec2::splat(footprint);
            draw_control_rect(
                painter,
                self.screen_map(context),
                shrimply_preview_provider_skia::Rect {
                    min: self.origin() + shadow - self.center() - footprint,
                    max: self.origin() + self.size() + shadow - self.center() + footprint,
                },
                Color::new(0.86, 0.31, 1.0, 0.43),
            );
        }
    }

    fn draw_local_line(
        &self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        start: Vec2,
        end: Vec2,
        color: Color,
        context: &dyn PreviewContext,
    ) {
        draw_control_line(
            painter,
            self.local_to_screen(start, context),
            self.local_to_screen(end, context),
            color,
        );
    }

    fn draw_local_circle(
        &self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        center: Vec2,
        radius: f32,
        color: Color,
        context: &dyn PreviewContext,
    ) {
        let edge = self.local_to_screen(center + Vec2::X * radius, context);
        let center = self.local_to_screen(center, context);
        shrimply_preview_provider_skia::drawing::circle(
            painter,
            center,
            center.distance(edge),
            Paint::stroke(Stroke::new(color, GUIDE_WIDTH)),
        );
    }
}

impl PreviewProvider for ShapeHandler {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        if self.target.facet() == CONTENT_FACET {
            self.draw_content(painter, context);
        } else {
            self.draw_appearance(painter, context);
        }
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        match event {
            shrimply_preview_provider_skia::PointerEvent::Hover(input) => {
                let hit = self.hit(input.sample.position, context);
                PreviewResponse {
                    handled: hit.is_some(),
                    redraw: false,
                    cursor: hit.map_or(CursorUpdate::Clear, |_| CursorUpdate::Set(Cursor::Grab)),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            shrimply_preview_provider_skia::PointerEvent::Leave => PreviewResponse {
                cursor: CursorUpdate::Clear,
                ..PreviewResponse::IGNORED
            },
            shrimply_preview_provider_skia::PointerEvent::Begin(input) => {
                let Some(control) = self.hit(input.sample.position, context) else {
                    return PreviewResponse::IGNORED;
                };
                let snapshot = edits
                    .target_mut(self.target)
                    .downcast_ref::<ShapeItem>()
                    .expect("shape preview target has the wrong type")
                    .clone();
                self.drag = Some(DragState {
                    control,
                    changed: false,
                    snapshot,
                    controls: self.controls,
                });
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Grabbing),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            shrimply_preview_provider_skia::PointerEvent::Samples { input, samples } => {
                let point = samples
                    .last()
                    .map_or(input.sample.position, |sample| sample.position);
                let changed = self.apply_drag(point, context, edits);
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::live(PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR)
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            shrimply_preview_provider_skia::PointerEvent::End(_) => {
                let Some(drag) = self.drag.take() else {
                    return PreviewResponse::IGNORED;
                };
                PreviewResponse {
                    handled: true,
                    redraw: drag.changed,
                    cursor: CursorUpdate::Clear,
                    edit: if drag.changed {
                        PreviewEditOutcome::committed(
                            PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                        )
                    } else {
                        PreviewEditOutcome::UNCHANGED
                    },
                }
            }
            shrimply_preview_provider_skia::PointerEvent::Cancel => {
                let Some(drag) = self.drag.take() else {
                    return PreviewResponse::IGNORED;
                };
                if drag.changed {
                    *edits
                        .target_mut(self.target)
                        .downcast_mut::<ShapeItem>()
                        .expect("shape preview target has the wrong type") = drag.snapshot;
                    self.controls = drag.controls;
                }
                PreviewResponse {
                    handled: true,
                    redraw: drag.changed,
                    cursor: CursorUpdate::Clear,
                    edit: if drag.changed {
                        PreviewEditOutcome::live(
                            PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                        )
                    } else {
                        PreviewEditOutcome::UNCHANGED
                    },
                }
            }
            _ => PreviewResponse::IGNORED,
        }
    }
}

#[derive(Clone, Copy)]
struct EllipseGeometry {
    center: Vec2,
    radius: Vec2,
    start_radians: f32,
    sweep_radians: f32,
}
