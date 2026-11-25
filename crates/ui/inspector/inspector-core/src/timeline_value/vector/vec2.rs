use glam::Vec2;
use shrimply_project_document::project::Time;
use shrimply_property_model::timeline_value::{
    CurveEditPolicy, CurveKeyframeInsert, TimelineValue, edit_curve_value,
};

use crate::keyframe_model;

const VALUE_EPSILON_SQUARED: f32 = 0.000_001;

pub type Vec2Timeline = TimelineValue<Vec2>;

pub fn value_at(value: &Vec2Timeline, time: Time) -> Vec2 {
    value.value_at(time)
}

pub fn set_component(
    value: &mut Vec2Timeline,
    evaluation_time: Time,
    keyframe_time: Time,
    component: usize,
    next: f64,
) -> bool {
    if !next.is_finite() {
        return false;
    }
    let mut value_at_time = value_at(value, evaluation_time);
    match component {
        0 => value_at_time.x = next as f32,
        1 => value_at_time.y = next as f32,
        _ => return false,
    }
    set_value(value, keyframe_time, value_at_time)
}

pub fn set_value(value: &mut Vec2Timeline, time: Time, next: Vec2) -> bool {
    edit_curve_value(
        value,
        time,
        next,
        |current, next| (*current - *next).length_squared() <= VALUE_EPSILON_SQUARED,
        CurveEditPolicy {
            unchanged_keyframe_is_noop: false,
            insert: CurveKeyframeInsert::InheritPreviousInterpolation,
        },
    )
}

pub fn set_keyframes_enabled(
    value: &mut Vec2Timeline,
    evaluation_time: Time,
    keyframe_time: Time,
    enabled: bool,
) -> bool {
    let current = value_at(value, evaluation_time);
    keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled)
}

pub fn set_expression_enabled(value: &mut Vec2Timeline, enabled: bool) -> bool {
    keyframe_model::set_expression_enabled(
        value,
        enabled,
        crate::timeline_value::VECTOR2_EXPRESSION_DEFAULT,
    )
}

pub use shrimply_property_model::timeline_value::set_expression_source;

pub use crate::keyframe_model::{
    copy_keyframes, move_discrete_keyframes as move_keyframes, paste_keyframes,
};

pub use crate::keyframe_model::set_interpolation;

pub fn format_value(
    value: Vec2,
    first_prefix: &str,
    second_prefix: &str,
    digits: usize,
    unit: &str,
) -> String {
    format!(
        "{} {:.*}{}  {} {:.*}{}",
        first_prefix, digits, value.x, unit, second_prefix, digits, value.y, unit,
    )
}
