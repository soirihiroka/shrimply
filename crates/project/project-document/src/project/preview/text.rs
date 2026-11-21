use glam::{Mat3, Vec2};
use shrimply_preview_provider_skia::{
    Color, Cursor, CursorUpdate, PointerEvent, PreviewBuilder, PreviewContext, PreviewEditOutcome,
    PreviewEditSink, PreviewFacetKey, PreviewItemGeometry, PreviewProvider, PreviewRefresh,
    PreviewResponse, PreviewTarget, Rect, draw_control_line, draw_control_rect, draw_keypoint,
    hit_keypoint,
};

use super::super::TextItem;

pub const APPEARANCE_FACET: PreviewFacetKey = PreviewFacetKey::new("text.appearance");

const SHADOW_CONTROL_COLOR: Color = Color::new(0.86, 0.31, 1.0, 1.0);

pub fn size(text: &TextItem, builder: &impl PreviewBuilder) -> Vec2 {
    let content = builder.resolve(&text.text);
    let font_size = builder.resolve(&text.font_size).max(1.0);
    let tracking = builder.resolve(&text.tracking);
    let line_height = builder.resolve(&text.line_height).max(0.1);
    let (lines, characters) = content.lines().fold((0usize, 0usize), |current, line| {
        (current.0 + 1, current.1.max(line.chars().count()))
    });
    let characters = characters.max(1) as f32;
    Vec2::new(
        characters * font_size * 0.6 + (characters - 1.0) * tracking,
        lines.max(1) as f32 * font_size * line_height,
    )
    .max(Vec2::ONE)
}

pub(super) fn bounds(
    text: &TextItem,
    size: Vec2,
    rotation_degrees: f32,
    builder: &impl PreviewBuilder,
) -> (Rect, Vec2, Vec2) {
    let anchor = Vec2::new(
        match builder.resolve(&text.h_align) {
            super::super::TextHorizontalAlign::Left => 0.0,
            super::super::TextHorizontalAlign::Center | super::super::TextHorizontalAlign::Fill => {
                size.x * 0.5
            }
            super::super::TextHorizontalAlign::Right => size.x,
        },
        match builder.resolve(&text.v_align) {
            super::super::VerticalAlign::Top => 0.0,
            super::super::VerticalAlign::Middle => size.y * 0.5,
            super::super::VerticalAlign::Bottom => size.y,
        },
    );
    let content = Rect::from_min_size(-anchor, size);
    let angle = (builder.resolve(&text.shadow_direction_degrees) - rotation_degrees).to_radians();
    let shadow_offset =
        Vec2::new(angle.cos(), angle.sin()) * builder.resolve(&text.shadow_distance).max(0.0);
    let padding = builder.resolve(&text.background_padding).max(Vec2::ZERO);
    let bounds = super::math::decorated_bounds(
        content,
        builder.resolve(&text.outline_width),
        shadow_offset,
        builder.resolve(&text.shadow_width),
        builder.resolve(&text.shadow_blur),
        padding,
    );
    (bounds, bounds.size() - content.size(), anchor)
}

pub fn provider(
    text: &TextItem,
    target: PreviewTarget,
    geometry: PreviewItemGeometry,
    builder: &impl PreviewBuilder,
) -> Box<dyn PreviewProvider> {
    Box::new(TextAppearanceHandler {
        target,
        snapshot: text.clone(),
        geometry,
        outline_width: builder.resolve(&text.outline_width).max(0.0),
        shadow_distance: builder.resolve(&text.shadow_distance).max(0.0),
        shadow_direction_degrees: builder.resolve(&text.shadow_direction_degrees),
        shadow_width: builder.resolve(&text.shadow_width).max(0.0),
        shadow_blur: builder.resolve(&text.shadow_blur).max(0.0),
        distance_editable: super::visual_item::editable(&text.shadow_distance),
        direction_editable: super::visual_item::editable(&text.shadow_direction_degrees),
        drag: None,
    })
}

struct TextAppearanceHandler {
    target: PreviewTarget,
    snapshot: TextItem,
    geometry: PreviewItemGeometry,
    outline_width: f32,
    shadow_distance: f32,
    shadow_direction_degrees: f32,
    shadow_width: f32,
    shadow_blur: f32,
    distance_editable: bool,
    direction_editable: bool,
    drag: Option<bool>,
}

impl TextAppearanceHandler {
    fn screen_map(&self, context: &dyn PreviewContext) -> Mat3 {
        context.viewport().canvas_to_screen * self.geometry.local_to_canvas
    }

    fn shadow_offset(&self) -> Vec2 {
        let angle =
            (self.shadow_direction_degrees - self.geometry.transform.rotation_degrees).to_radians();
        Vec2::new(angle.cos(), angle.sin()) * self.shadow_distance
    }

    fn shadow_handle(&self, context: &dyn PreviewContext) -> Vec2 {
        self.screen_map(context).transform_point2(
            -self.geometry.anchor_offset + self.geometry.source_size * 0.5 + self.shadow_offset(),
        )
    }

    fn hit(&self, point: Vec2, context: &dyn PreviewContext) -> bool {
        (self.distance_editable || self.direction_editable)
            && hit_keypoint(point, self.shadow_handle(context))
    }

    fn drag(
        &mut self,
        point: Vec2,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> bool {
        if self.drag.is_none() {
            return false;
        }
        let map = self.screen_map(context);
        let determinant = map.determinant();
        if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
            return false;
        }

        let content_center = -self.geometry.anchor_offset + self.geometry.source_size * 0.5;
        let offset = map.inverse().transform_point2(point) - content_center;
        let distance = offset.length();
        let direction = (offset.length_squared() > f32::EPSILON).then(|| {
            offset.y.atan2(offset.x).to_degrees() + self.geometry.transform.rotation_degrees
        });
        let time = edits.keyframe_time();
        let text = edits
            .target_mut(self.target)
            .downcast_mut::<TextItem>()
            .expect("text appearance preview target has the wrong type");
        let changed = (self.distance_editable
            && super::visual_item::set_scalar(&mut text.shadow_distance, time, distance))
            | (self.direction_editable
                && direction.is_some_and(|direction| {
                    super::visual_item::set_scalar(
                        &mut text.shadow_direction_degrees,
                        time,
                        direction,
                    )
                }));
        if changed {
            if self.distance_editable {
                self.shadow_distance = distance;
            }
            if self.direction_editable
                && let Some(direction) = direction
            {
                self.shadow_direction_degrees = direction;
            }
            self.drag = Some(true);
        }
        changed
    }

    fn cancel(&mut self, edits: &mut dyn PreviewEditSink) -> bool {
        let changed = self.drag.take().is_some_and(|changed| changed);
        if changed {
            *edits
                .target_mut(self.target)
                .downcast_mut::<TextItem>()
                .expect("text appearance preview target has the wrong type") =
                self.snapshot.clone();
        }
        changed
    }
}

impl PreviewProvider for TextAppearanceHandler {
    fn on_draw(
        &mut self,
        painter: &shrimply_preview_provider_skia::PreviewCanvas,
        context: &dyn PreviewContext,
    ) {
        let map = self.screen_map(context);
        let content_bounds =
            Rect::from_min_size(-self.geometry.anchor_offset, self.geometry.source_size);
        let outline = Vec2::splat(self.outline_width);
        draw_control_rect(
            painter,
            map,
            Rect {
                min: content_bounds.min - outline,
                max: content_bounds.max + outline,
            },
            shrimply_preview_provider_skia::selection_color(),
        );

        let center = map.transform_point2(content_bounds.center());
        let offset = self.shadow_offset();
        let shadow = map.transform_point2(content_bounds.center() + offset);
        let footprint = Vec2::splat(self.shadow_width + self.shadow_blur);
        draw_control_rect(
            painter,
            map,
            Rect {
                min: content_bounds.min + offset - footprint,
                max: content_bounds.max + offset + footprint,
            },
            SHADOW_CONTROL_COLOR,
        );
        draw_control_line(painter, center, shadow, SHADOW_CONTROL_COLOR);
        draw_keypoint(painter, shadow, SHADOW_CONTROL_COLOR);
    }

    fn on_pointer(
        &mut self,
        event: PointerEvent<'_>,
        context: &dyn PreviewContext,
        edits: &mut dyn PreviewEditSink,
    ) -> PreviewResponse {
        match event {
            PointerEvent::Hover(input) => PreviewResponse {
                handled: self.hit(input.sample.position, context),
                redraw: false,
                cursor: if self.hit(input.sample.position, context) {
                    CursorUpdate::Set(Cursor::Grab)
                } else {
                    CursorUpdate::Clear
                },
                edit: PreviewEditOutcome::UNCHANGED,
            },
            PointerEvent::Leave => PreviewResponse {
                cursor: CursorUpdate::Clear,
                ..PreviewResponse::IGNORED
            },
            PointerEvent::Begin(input) => {
                if !self.hit(input.sample.position, context) {
                    return PreviewResponse::IGNORED;
                }
                self.drag = Some(false);
                PreviewResponse {
                    handled: true,
                    redraw: false,
                    cursor: CursorUpdate::Set(Cursor::Grabbing),
                    edit: PreviewEditOutcome::UNCHANGED,
                }
            }
            PointerEvent::Samples { input, samples } => {
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
            PointerEvent::End(_) => {
                let changed = self.drag.take().is_some_and(|changed| changed);
                PreviewResponse::edited(if changed {
                    PreviewEditOutcome::committed(
                        PreviewRefresh::PREVIEW | PreviewRefresh::INSPECTOR,
                    )
                } else {
                    PreviewEditOutcome::UNCHANGED
                })
            }
            PointerEvent::Cancel => {
                let changed = self.cancel(edits);
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
