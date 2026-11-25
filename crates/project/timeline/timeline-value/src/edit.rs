use crate::{
    Time, TimelineBase, TimelineCurveKeyframe, TimelineExpression, TimelineKeyframe, TimelineValue,
    TimelineValueType,
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CurveKeyframeInsert {
    Default,
    InheritPreviousInterpolation,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CurveEditPolicy {
    pub unchanged_keyframe_is_noop: bool,
    pub insert: CurveKeyframeInsert,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscreteEditPolicy {
    pub unchanged_is_noop: bool,
    pub sort_updated_keyframe: bool,
}

pub fn set_keyframes_enabled<T>(
    value: &mut TimelineValue<T>,
    time: Time,
    current: T,
    enabled: bool,
) -> bool
where
    T: TimelineValueType,
{
    match (&mut value.base, enabled) {
        (TimelineBase::Const(_), false) | (TimelineBase::Keyframes(_), true) => false,
        (base @ TimelineBase::Const(_), true) => {
            *base = TimelineBase::Keyframes(vec![T::keyframe(time, current)]);
            true
        }
        (base @ TimelineBase::Keyframes(_), false) => {
            *base = TimelineBase::Const(current);
            true
        }
    }
}

pub fn set_expression_enabled<T>(
    value: &mut TimelineValue<T>,
    enabled: bool,
    default_source: &str,
) -> bool
where
    T: TimelineValueType,
{
    match &mut value.expression {
        Some(expression) => expression.update(None, Some(enabled)),
        None if !enabled => false,
        None => {
            value.expression = Some(TimelineExpression {
                id: Uuid::new_v4(),
                enabled: true,
                source: default_source.to_string(),
            });
            true
        }
    }
}

pub fn set_expression_source<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    source: String,
) -> bool {
    value
        .expression
        .as_mut()
        .is_some_and(|expression| expression.update(Some(source), None))
}

pub fn remove_keyframes<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    mut remove: impl FnMut(&T::Keyframe) -> bool,
    fallback: T,
) -> bool {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let previous_len = keyframes.len();
    keyframes.retain(|keyframe| !remove(keyframe));
    let changed = keyframes.len() != previous_len;
    if changed && keyframes.is_empty() {
        value.base = TimelineBase::Const(fallback);
    }
    changed
}

pub fn edit_curve_value<T>(
    value: &mut TimelineValue<T>,
    time: Time,
    next: T,
    equivalent: impl Fn(&T, &T) -> bool,
    policy: CurveEditPolicy,
) -> bool
where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
{
    match &mut value.base {
        TimelineBase::Const(current) if equivalent(current, &next) => false,
        TimelineBase::Const(current) => {
            *current = next;
            true
        }
        TimelineBase::Keyframes(keyframes) => {
            if let Some(keyframe) = keyframes
                .iter_mut()
                .find(|keyframe| keyframe.time.approx_eq(time))
            {
                if policy.unchanged_keyframe_is_noop
                    && keyframe.time == time
                    && equivalent(&keyframe.value, &next)
                {
                    return false;
                }
                keyframe.time = time;
                keyframe.value = next;
                keyframes.sort_by_key(|keyframe| keyframe.time);
                return true;
            }
            let keyframe = match policy.insert {
                CurveKeyframeInsert::Skip => return false,
                CurveKeyframeInsert::Default
                | CurveKeyframeInsert::InheritPreviousInterpolation => T::keyframe(time, next),
            };
            insert_curve_keyframe(keyframes, keyframe, policy.insert);
            true
        }
    }
}

pub fn add_curve_keyframe<T>(
    value: &mut TimelineValue<T>,
    time: Time,
    normalize: impl FnOnce(T) -> T,
) -> bool
where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
{
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return false;
    };
    let current = keyframes
        .iter()
        .find(|keyframe| keyframe.time.approx_eq(time))
        .map_or_else(|| value.value_at(time), |keyframe| keyframe.value.clone());
    edit_curve_value(
        value,
        time,
        normalize(current),
        PartialEq::eq,
        CurveEditPolicy {
            unchanged_keyframe_is_noop: true,
            insert: CurveKeyframeInsert::InheritPreviousInterpolation,
        },
    )
}

pub fn insert_curve_keyframe<T>(
    keyframes: &mut Vec<TimelineCurveKeyframe<T>>,
    mut keyframe: TimelineCurveKeyframe<T>,
    insert: CurveKeyframeInsert,
) where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
{
    if insert == CurveKeyframeInsert::InheritPreviousInterpolation
        && let Some(previous) = keyframes
            .iter()
            .rev()
            .find(|previous| previous.time < keyframe.time)
        && keyframes
            .iter()
            .any(|following| following.time > keyframe.time)
    {
        keyframe.interpolation_to_next = previous.interpolation_to_next;
    }
    keyframes.push(keyframe);
    keyframes.sort_by_key(|keyframe| keyframe.time);
}

pub fn edit_discrete_value<T>(
    value: &mut TimelineValue<T>,
    time: Time,
    next: T,
    same_time: impl Fn(Time, Time) -> bool,
    policy: DiscreteEditPolicy,
) -> bool
where
    T: TimelineValueType,
{
    match &mut value.base {
        TimelineBase::Const(current) if policy.unchanged_is_noop && *current == next => false,
        TimelineBase::Const(current) => {
            *current = next;
            true
        }
        TimelineBase::Keyframes(keyframes) => {
            if let Some(keyframe) = keyframes
                .iter_mut()
                .find(|keyframe| same_time(keyframe.time(), time))
            {
                if policy.unchanged_is_noop && keyframe.time() == time && keyframe.value() == &next
                {
                    return false;
                }
                *keyframe.time_mut() = time;
                *keyframe.value_mut() = next;
                if policy.sort_updated_keyframe {
                    keyframes.sort_by_key(TimelineKeyframe::time);
                }
            } else {
                keyframes.push(T::keyframe(time, next));
                keyframes.sort_by_key(TimelineKeyframe::time);
            }
            true
        }
    }
}
