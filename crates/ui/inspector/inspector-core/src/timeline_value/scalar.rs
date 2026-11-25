use shrimply_project_document::project::Time;
use shrimply_property_model::timeline_value::{TimelineBase, TimelineValue};

use crate::NumberConstraint;

#[derive(Clone, Copy)]
pub enum ScalarConstraint {
    Function(fn(f32) -> f32),
    Number(NumberConstraint),
}

impl ScalarConstraint {
    pub fn apply(self, value: f32) -> f32 {
        match self {
            Self::Function(clamp) => clamp(value),
            Self::Number(constraint) => constraint.clamp_f32(value),
        }
    }

    pub fn apply_structural(self, value: f32) -> f32 {
        match self {
            Self::Function(_) => value,
            Self::Number(constraint) => constraint.clamp_f32(value),
        }
    }
}

impl From<NumberConstraint> for ScalarConstraint {
    fn from(value: NumberConstraint) -> Self {
        Self::Number(value)
    }
}

pub fn set_displayed_value(
    value: &mut TimelineValue<f32>,
    time: Time,
    displayed: f64,
    store: fn(f64) -> f32,
    constraint: ScalarConstraint,
) -> bool {
    crate::keyframe_model::set_scalar_value(value, time, constraint.apply(store(displayed)))
}

pub fn set_keyframes_enabled(
    value: &mut TimelineValue<f32>,
    evaluation_time: Time,
    keyframe_time: Time,
    enabled: bool,
    constraint: ScalarConstraint,
) -> bool {
    let current = constraint.apply_structural(value.value_at(evaluation_time));
    crate::keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled)
}

pub use shrimply_property_model::timeline_value::set_expression_source;

pub fn move_displayed_keyframe(
    value: &mut TimelineValue<f32>,
    old_time: Time,
    time: Time,
    displayed: f64,
    store: fn(f64) -> f32,
    constraint: ScalarConstraint,
) -> bool {
    move_keyframe(value, old_time, time, store(displayed), constraint)
}

pub fn move_stored_keyframe(
    value: &mut TimelineValue<f32>,
    old_time: Time,
    time: Time,
    stored: f32,
    constraint: NumberConstraint,
) -> bool {
    move_keyframe(value, old_time, time, stored, constraint.into())
}

pub fn move_stored_keyframes(
    value: &mut TimelineValue<f32>,
    changes: &[(Time, Time, f32)],
    constraint: NumberConstraint,
) -> bool {
    let changes = changes
        .iter()
        .map(|(old, time, stored)| (*old, *time, constraint.clamp_f32(*stored)))
        .collect::<Vec<_>>();
    if changes.iter().any(|(_, _, stored)| !stored.is_finite()) {
        return false;
    }
    crate::keyframe_model::update_keyframes(value, &changes)
}

fn move_keyframe(
    value: &mut TimelineValue<f32>,
    old_time: Time,
    time: Time,
    stored: f32,
    constraint: ScalarConstraint,
) -> bool {
    crate::keyframe_model::update_scalar_keyframe(value, old_time, time, constraint.apply(stored))
}

pub fn constrain_keyframes(
    value: &mut TimelineValue<f32>,
    times: &[Time],
    constraint: ScalarConstraint,
) {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return;
    };
    for keyframe in keyframes
        .iter_mut()
        .filter(|keyframe| times.contains(&keyframe.time))
    {
        keyframe.value = constraint.apply_structural(keyframe.value);
    }
}
