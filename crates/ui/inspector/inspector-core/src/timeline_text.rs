use shrimply_project_document::project::{Time, VideoItem};
use shrimply_property_model::timeline_value::{
    DiscreteEditPolicy, Interpolation, TextInterpolation, TimelineBase, TimelineValue,
    edit_discrete_value, text_edit_count,
};
use uuid::Uuid;

use crate::keyframe_graph::{KeyframeGraph, SpeedSegment};
use crate::{GraphPoint, GraphSegment, InspectorRuntime, ScalarGraph};

pub type TextTimeline = TimelineValue<String>;

pub fn video_value<'a>(
    item: &'a VideoItem,
    path: &str,
    timeline_id: Uuid,
) -> Option<&'a TextTimeline> {
    crate::generated::text_value(item, path, timeline_id)
        .or_else(|| crate::visual_modifiers::visual_modifier_text(item, path, timeline_id))
}

pub fn value_at(value: &TextTimeline, time: Time) -> String {
    value.value_at(time)
}

pub fn set_value(value: &mut TextTimeline, time: Time, next: String, frame_step: Time) -> bool {
    edit_discrete_value(
        value,
        time,
        next,
        |left, right| crate::keyframe_model::same_frame(left, right, frame_step),
        DiscreteEditPolicy {
            unchanged_is_noop: true,
            sort_updated_keyframe: true,
        },
    )
}

pub fn set_keyframes_enabled(
    value: &mut TextTimeline,
    evaluation_time: Time,
    keyframe_time: Time,
    enabled: bool,
) -> bool {
    let current = value_at(value, evaluation_time);
    crate::keyframe_model::set_keyframes_enabled(value, keyframe_time, current, enabled)
}

pub fn set_expression_enabled(value: &mut TextTimeline, enabled: bool) -> bool {
    crate::keyframe_model::set_expression_enabled(
        value,
        enabled,
        crate::timeline_value::SCALAR_EXPRESSION_DEFAULT,
    )
}

pub use shrimply_property_model::timeline_value::set_expression_source;

pub fn add_keyframe(value: &mut TextTimeline, time: Time, frame_step: Time) -> bool {
    let current = value_at(value, time);
    if !matches!(&value.base, TimelineBase::Keyframes(_)) {
        return false;
    }
    set_value(value, time, current, frame_step)
}

pub fn delete_keyframe(value: &mut TextTimeline, time: Time, frame_step: Time) -> bool {
    crate::keyframe_model::delete_discrete_keyframe(value, time, frame_step)
}

pub fn move_keyframes(value: &mut TextTimeline, moves: &[(Time, Time)]) -> bool {
    crate::keyframe_model::move_discrete_keyframes(value, moves)
}

pub fn copy_keyframes(
    value: &TextTimeline,
    selected: &[Time],
) -> Option<crate::keyframe_model::KeyframeClipboard> {
    crate::keyframe_model::copy_keyframes(value, selected)
}

pub fn paste_keyframes(
    value: &mut TextTimeline,
    clipboard: &crate::keyframe_model::KeyframeClipboard,
    times: &[Time],
) -> Option<Vec<Time>> {
    crate::keyframe_model::paste_keyframes(value, clipboard, times)
}

pub fn set_interpolation(
    value: &mut TextTimeline,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> Result<bool, String> {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return Err("text keyframes are no longer enabled".to_string());
    };
    let keyframe = keyframes
        .iter_mut()
        .find(|keyframe| keyframe.id == owner_id)
        .ok_or_else(|| "text keyframe is no longer available".to_string())?;
    if keyframe.interpolation_to_next == interpolation {
        return Ok(false);
    }
    keyframe.interpolation_to_next = interpolation;
    Ok(true)
}

pub fn text_interpolation(value: &TextTimeline, owner_id: Uuid) -> Option<TextInterpolation> {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return None;
    };
    keyframes
        .iter()
        .find(|keyframe| keyframe.id == owner_id)
        .map(|keyframe| keyframe.text_interpolation_to_next)
}

pub fn set_text_interpolation(
    value: &mut TextTimeline,
    owner_id: Uuid,
    interpolation: TextInterpolation,
) -> Result<bool, String> {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return Err("text keyframes are no longer enabled".to_string());
    };
    let keyframe = keyframes
        .iter_mut()
        .find(|keyframe| keyframe.id == owner_id)
        .ok_or_else(|| "text keyframe is no longer available".to_string())?;
    if keyframe.text_interpolation_to_next == interpolation {
        return Ok(false);
    }
    keyframe.text_interpolation_to_next = interpolation;
    Ok(true)
}

pub fn keyframe_graph(value: &TextTimeline) -> KeyframeGraph {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return KeyframeGraph::Speed {
            segments: Vec::new(),
            keys: Vec::new(),
            static_value: 0.0,
        };
    };
    KeyframeGraph::Speed {
        segments: keyframes
            .windows(2)
            .filter_map(|pair| {
                let seconds = pair[1].time.signed_sub(pair[0].time).as_secs_f64();
                (seconds > f64::EPSILON).then(|| SpeedSegment {
                    owner_id: pair[0].id,
                    start: pair[0].time,
                    end: pair[1].time,
                    value: text_edit_count(
                        &pair[0].value,
                        &pair[1].value,
                        pair[0].text_interpolation_to_next,
                    ) as f64
                        / seconds,
                    interpolation: pair[0].interpolation_to_next,
                })
            })
            .collect(),
        keys: keyframes.iter().map(|keyframe| keyframe.time).collect(),
        static_value: 0.0,
    }
}

pub fn speed_graph(value: &TextTimeline, runtime: InspectorRuntime) -> Option<ScalarGraph> {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return None;
    };
    Some(ScalarGraph {
        points: keyframes
            .iter()
            .map(|keyframe| GraphPoint {
                time: keyframe.time,
                value: 0.0,
            })
            .collect(),
        segments: keyframes
            .windows(2)
            .filter_map(|pair| {
                let seconds = pair[1].time.signed_sub(pair[0].time).as_secs_f64();
                (seconds > f64::EPSILON).then(|| {
                    let speed = text_edit_count(
                        &pair[0].value,
                        &pair[1].value,
                        pair[0].text_interpolation_to_next,
                    ) as f64
                        / seconds;
                    GraphSegment {
                        owner_id: pair[0].id,
                        start: pair[0].time,
                        end: pair[1].time,
                        start_value: speed,
                        end_value: speed,
                        interpolation: Interpolation::KEYFRAME
                            .iter()
                            .position(|candidate| *candidate == pair[0].interpolation_to_next)
                            .expect("text interpolation must be available"),
                    }
                })
            })
            .collect(),
        range: runtime.keyframe_range.unwrap_or((Time::ZERO, Time::ZERO)),
        frame_step: runtime.frame_step,
        playhead: runtime.keyframe_playhead.unwrap_or(Time::ZERO),
    })
}
