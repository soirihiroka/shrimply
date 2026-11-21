use glam::{Mat3, Vec2};
pub use shrimply_preview_provider_skia::{
    BOUNDS_HANDLES as HANDLES, BoundsHandle as Handle, CONTROL_LINE_WIDTH as LINE_WIDTH,
    bounds_handle_cursor as resize_cursor, bounds_handle_position as handle_point, draw_anchor,
    draw_bounds_handles, draw_control_line as draw_line, draw_control_rect as draw_rect,
    draw_keypoint as draw_handle, draw_rotation_handle, hit_keypoint as hit,
    rotation_handle_segment, selection_color,
};
use shrimply_preview_provider_skia::{PreviewContext, PreviewFacetKey, PreviewTarget};
use shrimply_property_model::timeline_value::{
    Time, TimelineBase, TimelineCurveKeyframe, TimelineValue, TimelineValueType,
};
use uuid::Uuid;

pub const MODIFIER_PREVIEW_FACET: PreviewFacetKey = PreviewFacetKey::new("modifier-controls");
pub fn is_target(target: PreviewTarget) -> bool {
    target.facet() == MODIFIER_PREVIEW_FACET
}

pub fn editable<T: TimelineValueType>(value: &TimelineValue<T>) -> bool {
    !value
        .expression
        .as_ref()
        .is_some_and(|expression| expression.enabled)
}

pub fn screen_map(target: PreviewTarget, context: &dyn PreviewContext) -> Option<(Mat3, Vec2)> {
    let geometry = context.target_geometry(target)?;
    Some((
        context.viewport().canvas_to_screen * geometry.local_to_canvas,
        geometry.source_size,
    ))
}

pub fn inverse_point(map: Mat3, point: Vec2) -> Option<Vec2> {
    let determinant = map.determinant();
    if !determinant.is_finite() || determinant.abs() <= f32::EPSILON {
        return None;
    }
    let point = map.inverse().transform_point2(point);
    point.is_finite().then_some(point)
}

pub fn handle_axes(handle: Handle) -> (i8, i8) {
    match handle {
        Handle::TopLeft => (-1, -1),
        Handle::Top => (0, -1),
        Handle::TopRight => (1, -1),
        Handle::Right => (1, 0),
        Handle::BottomRight => (1, 1),
        Handle::Bottom => (0, 1),
        Handle::BottomLeft => (-1, 1),
        Handle::Left => (-1, 0),
    }
}

pub fn set_scalar(value: &mut TimelineValue<f32>, time: Time, next: f32) -> bool {
    if !editable(value) || !next.is_finite() {
        return false;
    }
    set_curve(value, time, next, |left, right| {
        (left - right).abs() <= 0.000_001
    })
}

pub fn set_vec2(value: &mut TimelineValue<Vec2>, time: Time, next: Vec2) -> bool {
    if !editable(value) || !next.is_finite() {
        return false;
    }
    set_curve(value, time, next, |left, right| {
        (left - right).length_squared() <= 0.000_001
    })
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
                let mut keyframe = TimelineCurveKeyframe {
                    id: Uuid::new_v4(),
                    time,
                    value: next,
                    interpolation_to_next: Default::default(),
                };
                if let Some(previous) = keyframes.iter().rev().find(|keyframe| keyframe.time < time)
                    && keyframes.iter().any(|keyframe| keyframe.time > time)
                {
                    keyframe.interpolation_to_next = previous.interpolation_to_next;
                }
                keyframes.push(keyframe);
                keyframes.sort_by_key(|keyframe| keyframe.time);
            }
        }
    }
    true
}
