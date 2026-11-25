use glam::Vec3;
use shrimply_project_document::project::Time;
use shrimply_property_model::timeline_value::{
    CurveEditPolicy, CurveKeyframeInsert, TimelineValue, edit_curve_value,
};

pub type Vec3Timeline = TimelineValue<Vec3>;
pub const COMPONENT_COUNT: usize = 3;

pub fn value_at(value: &Vec3Timeline, time: Time) -> Vec3 {
    value.value_at(time)
}

pub fn set_component(
    value: &mut Vec3Timeline,
    evaluation_time: Time,
    keyframe_time: Time,
    component: usize,
    next: f64,
    minimum: Option<f64>,
) -> bool {
    if !next.is_finite() {
        return false;
    }
    let mut components = value_at(value, evaluation_time).to_array();
    let Some(component) = components.get_mut(component) else {
        return false;
    };
    *component = next as f32;
    let mut current = Vec3::from_array(components);
    if let Some(minimum) = minimum {
        current = current.max(Vec3::splat(minimum as f32));
    }
    set_value(value, keyframe_time, current)
}

pub fn set_value(value: &mut Vec3Timeline, time: Time, next: Vec3) -> bool {
    if !next.is_finite() {
        return false;
    }
    edit_curve_value(
        value,
        time,
        next,
        |current, next| current.abs_diff_eq(*next, 0.000_001),
        CurveEditPolicy {
            unchanged_keyframe_is_noop: false,
            insert: CurveKeyframeInsert::InheritPreviousInterpolation,
        },
    )
}

pub fn set_keyframes_enabled(
    value: &mut Vec3Timeline,
    evaluation_time: Time,
    keyframe_time: Time,
    enabled: bool,
) -> bool {
    let current = value_at(value, evaluation_time);
    crate::keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled)
}

pub fn set_expression_enabled(value: &mut Vec3Timeline, enabled: bool) -> bool {
    crate::keyframe_model::set_expression_enabled(
        value,
        enabled,
        crate::timeline_value::VECTOR3_EXPRESSION_DEFAULT,
    )
}

pub use shrimply_property_model::timeline_value::set_expression_source;

pub use crate::keyframe_model::{
    copy_keyframes, move_discrete_keyframes as move_keyframes, paste_keyframes,
};

pub use crate::keyframe_model::set_interpolation;

pub fn format_value(
    value: Vec3,
    prefixes: [&str; COMPONENT_COUNT],
    digits: usize,
    unit: &str,
) -> String {
    format!(
        "{} {:.*}{}  {} {:.*}{}  {} {:.*}{}",
        prefixes[0],
        digits,
        value.x,
        unit,
        prefixes[1],
        digits,
        value.y,
        unit,
        prefixes[2],
        digits,
        value.z,
        unit,
    )
}
