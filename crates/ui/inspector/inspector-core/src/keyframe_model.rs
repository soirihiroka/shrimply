use std::{any::Any, cell::Ref, cell::RefCell, rc::Rc};

use shrimply_editor_state::player_state::ProjectChange;
use shrimply_project_document::project::{ItemAddress, Project, Time};
use shrimply_property_model::timeline_value::{
    CurveEditPolicy, CurveKeyframeInsert, DiscreteEditPolicy, Interpolation, TimelineBase,
    TimelineCurveKeyframe, TimelineKeyframe, TimelineValue, TimelineValueType, edit_curve_value,
    edit_discrete_value,
};
use uuid::Uuid;

use crate::keyframe_graph::{KeyframeGraph, KeyframePoint, RawSegment};

pub const KEYFRAME_CLIPBOARD_MARKER: &str = "shrimply keyframes";

pub fn graph_snapping(
    preferences: &shrimply_editor_state::preferences::SharedPreferences,
) -> (bool, f64) {
    let preferences = shrimply_editor_state::preferences::snapshot(preferences);
    (
        preferences.timeline_magnet == "true",
        f64::from(preferences.timeline_snap_radius_px),
    )
}

#[derive(Clone)]
pub struct KeyframeClipboard {
    values: Rc<dyn Any>,
    pub times: Vec<Time>,
    len: usize,
}

pub struct KeyframeClipboardCache(RefCell<Option<KeyframeClipboard>>);

impl KeyframeClipboardCache {
    pub const fn new() -> Self {
        Self(RefCell::new(None))
    }

    pub fn replace(&self, clipboard: Option<KeyframeClipboard>) {
        self.0.replace(clipboard);
    }

    pub fn borrow(&self) -> Ref<'_, Option<KeyframeClipboard>> {
        self.0.borrow()
    }
}

impl Default for KeyframeClipboardCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct JsonDiscreteClipboard {
    timeline_type: String,
    keyframes: Vec<serde_json::Value>,
}

impl KeyframeClipboard {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub fn project_frame_step(project: &Project, item: Option<&ItemAddress>) -> Time {
    item.and_then(|item| project.keyframe_step(item))
        .filter(|step| *step > Time::ZERO)
        .unwrap_or_else(|| project.frame_step())
}

pub fn canonical_keyframe_time(
    project: &Project,
    item: Option<&ItemAddress>,
    time: Time,
) -> Option<Time> {
    let Some(item) = item else {
        return Some(time.snapped(project.frame_step()));
    };
    project
        .keyframe_timeline_time(item, time)
        .and_then(|timeline_time| project.keyframe_time(item, timeline_time))
}

pub fn bounded_visible_area(
    project: &Project,
    item: Option<&ItemAddress>,
    (start, end): (Time, Time),
) -> (Time, Time) {
    let duration = item
        .and_then(|item| project.item(item))
        .map(|item| {
            let (start, end) = item.times();
            end.saturating_sub(start).max(start.saturating_sub(end))
        })
        .unwrap_or(Time::ZERO);
    (start, end.max(start.saturating_add(duration)))
}

pub fn normalize_clipboard_times(
    project: &Project,
    item: Option<&ItemAddress>,
    clipboard: &mut KeyframeClipboard,
) -> bool {
    let timeline_times = clipboard
        .times
        .iter()
        .map(|time| {
            item.and_then(|item| project.keyframe_timeline_time(item, *time))
                .unwrap_or(*time)
                .snapped(project.frame_step())
        })
        .collect::<Vec<_>>();
    let Some(origin) = timeline_times.first().copied() else {
        return false;
    };
    clipboard.times = timeline_times
        .into_iter()
        .map(|time| Time {
            seconds: time.seconds - origin.seconds,
        })
        .collect();
    true
}

pub fn clipboard_paste_times(
    project: &Project,
    item: Option<&ItemAddress>,
    clipboard: &KeyframeClipboard,
    time: Time,
) -> Option<Vec<Time>> {
    let anchor = item
        .and_then(|item| project.keyframe_timeline_time(item, time))
        .unwrap_or(time)
        .snapped(project.frame_step());
    let times = clipboard
        .times
        .iter()
        .filter_map(|offset| {
            let timeline_time = Time {
                seconds: anchor.seconds + offset.seconds,
            };
            item.map(|item| project.keyframe_time(item, timeline_time))
                .unwrap_or(Some(timeline_time))
        })
        .collect::<Vec<_>>();
    (times.len() == clipboard.len()).then_some(times)
}

pub fn exact_time(numerator: i64, denominator: i64) -> Result<Time, String> {
    if denominator <= 0 {
        return Err("keyframe graph time denominator must be positive".to_string());
    }
    Ok(Time::from_fraction(numerator, denominator))
}

pub fn parse_time(value: &str) -> Result<Time, String> {
    let (numerator, denominator) = value
        .split_once('/')
        .ok_or_else(|| format!("keyframe graph time is not an exact fraction: {value}"))?;
    exact_time(
        numerator
            .parse()
            .map_err(|_| format!("keyframe graph time numerator is invalid: {value}"))?,
        denominator
            .parse()
            .map_err(|_| format!("keyframe graph time denominator is invalid: {value}"))?,
    )
}

pub fn parse_graph_value(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("keyframe graph value is invalid: {value}"))
        .and_then(|value| {
            value
                .is_finite()
                .then_some(value)
                .ok_or_else(|| "keyframe graph value must be finite".to_string())
        })
}

pub fn parse_owner_id(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| "keyframe owner ID is invalid".to_string())
}

pub fn interpolation(index: usize) -> Result<Interpolation, String> {
    Interpolation::KEYFRAME
        .get(index)
        .copied()
        .ok_or_else(|| "keyframe interpolation is invalid".to_string())
}

pub fn set_keyframes_enabled<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    time: Time,
    current: T,
    enabled: bool,
) -> bool {
    shrimply_property_model::timeline_value::set_keyframes_enabled(value, time, current, enabled)
}

pub fn set_expression_enabled<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    enabled: bool,
    default_source: &str,
) -> bool {
    shrimply_property_model::timeline_value::set_expression_enabled(value, enabled, default_source)
}

pub fn set_json_keyframes_enabled(
    timeline: &mut serde_json::Value,
    time: Time,
    current: serde_json::Value,
    enabled: bool,
) -> Result<bool, String> {
    let base = timeline
        .get_mut("base")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "inspector timeline base is invalid".to_string())?;
    let currently_enabled = base.contains_key("keyframes");
    if currently_enabled == enabled {
        return Ok(false);
    }
    *base = if enabled {
        serde_json::json!({
            "keyframes": [{
                "id": Uuid::new_v4(),
                "time": time,
                "value": current,
                "interpolation_to_next": Interpolation::default(),
            }]
        })
        .as_object()
        .expect("timeline keyframe base must be an object")
        .clone()
    } else {
        serde_json::json!({ "const": current })
            .as_object()
            .expect("timeline constant base must be an object")
            .clone()
    };
    Ok(true)
}

pub fn copy_keyframes<T: TimelineValueType>(
    value: &TimelineValue<T>,
    selected: &[Time],
) -> Option<KeyframeClipboard> {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return None;
    };
    let copied: Vec<T::Keyframe> = keyframes
        .iter()
        .filter(|keyframe| selected.contains(&keyframe.time()))
        .cloned()
        .collect();
    (!copied.is_empty()).then(|| KeyframeClipboard {
        len: copied.len(),
        times: copied.iter().map(TimelineKeyframe::time).collect(),
        values: Rc::new(copied),
    })
}

pub fn paste_keyframes<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    clipboard: &KeyframeClipboard,
    times: &[Time],
) -> Option<Vec<Time>> {
    let source = clipboard.values.downcast_ref::<Vec<T::Keyframe>>()?;
    if source.len() != times.len() {
        return None;
    }
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return None;
    };
    let mut pasted = source.clone();
    for (keyframe, time) in pasted.iter_mut().zip(times) {
        *keyframe.id_mut() = Uuid::new_v4();
        *keyframe.time_mut() = *time;
    }
    let paste_start = *times.iter().min()?;
    let paste_end = *times.iter().max()?;
    if paste_start == paste_end {
        keyframes.retain(|keyframe| keyframe.time() != paste_start);
    } else {
        keyframes.retain(|keyframe| keyframe.time() < paste_start || keyframe.time() > paste_end);
    }
    let pasted_times = pasted.iter().map(TimelineKeyframe::time).collect();
    keyframes.extend(pasted);
    keyframes.sort_by_key(TimelineKeyframe::time);
    Some(pasted_times)
}

pub fn live_refresh(mut refresh: ProjectChange) -> ProjectChange {
    refresh.inspector = false;
    refresh.live_preview = true;
    refresh
}

pub fn same_frame(left: Time, right: Time, frame_step: Time) -> bool {
    if frame_step > Time::ZERO {
        left.snapped(frame_step) == right.snapped(frame_step)
    } else {
        left.approx_eq(right)
    }
}

pub fn scalar_graph(
    value: &TimelineValue<f32>,
    static_value: f64,
    display: impl Fn(f32) -> f64,
) -> KeyframeGraph {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return KeyframeGraph::RawValue {
            points: Vec::new(),
            segments: Vec::new(),
            static_value,
        };
    };
    KeyframeGraph::RawValue {
        points: keyframes
            .iter()
            .map(|keyframe| KeyframePoint {
                time: keyframe.time,
                value: display(keyframe.value),
            })
            .collect(),
        segments: keyframes
            .windows(2)
            .map(|pair| RawSegment {
                owner_id: pair[0].id,
                start: pair[0].time,
                end: pair[1].time,
                start_value: display(pair[0].value),
                end_value: display(pair[1].value),
                interpolation: pair[0].interpolation_to_next,
            })
            .collect(),
        static_value,
    }
}

pub fn step_graph<T: TimelineValueType>(value: &TimelineValue<T>) -> KeyframeGraph {
    step_graph_with(value, |_| 0.5)
}

pub fn step_graph_with<T: TimelineValueType>(
    value: &TimelineValue<T>,
    display: impl Fn(&T) -> f64,
) -> KeyframeGraph {
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return KeyframeGraph::Step { points: Vec::new() };
    };
    KeyframeGraph::Step {
        points: keyframes
            .iter()
            .map(|keyframe| KeyframePoint {
                time: keyframe.time(),
                value: display(keyframe.value()),
            })
            .collect(),
    }
}

pub fn set_discrete_value<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    time: Time,
    next: T,
    frame_step: Time,
) -> bool {
    edit_discrete_value(
        value,
        time,
        next,
        |left, right| same_frame(left, right, frame_step),
        DiscreteEditPolicy {
            unchanged_is_noop: false,
            sort_updated_keyframe: false,
        },
    )
}

pub fn add_discrete_keyframe<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    time: Time,
    frame_step: Time,
) -> bool {
    if !matches!(value.base, TimelineBase::Keyframes(_)) {
        return false;
    }
    let current = value.value_at(time);
    set_discrete_value(value, time, current, frame_step)
}

/// Applies a selection edit without exposing partially edited state on failure.
/// Callers replace/commit the resulting timeline once after this returns true.
pub fn edit_keyframe_selection<T: Clone>(
    value: &mut T,
    times: &[Time],
    mut edit: impl FnMut(&mut T, Time) -> Result<bool, String>,
) -> Result<bool, String> {
    if times.is_empty() {
        return Ok(false);
    }
    let mut next = value.clone();
    let mut changed = false;
    for &time in times {
        changed |= edit(&mut next, time)?;
    }
    if changed {
        *value = next;
    }
    Ok(changed)
}

pub fn delete_keyframes<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    times: &[Time],
    evaluation_time: Time,
    frame_step: Time,
) -> bool {
    let fallback = value.value_at(evaluation_time);
    shrimply_property_model::timeline_value::remove_keyframes(
        value,
        |keyframe| {
            times
                .iter()
                .any(|time| same_frame(keyframe.time(), *time, frame_step))
        },
        fallback,
    )
}

pub fn delete_discrete_keyframe<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    time: Time,
    frame_step: Time,
) -> bool {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let Some(index) = keyframes
        .iter()
        .position(|keyframe| same_frame(keyframe.time(), time, frame_step))
    else {
        return false;
    };
    let removed = keyframes.remove(index).value().clone();
    if keyframes.is_empty() {
        value.base = TimelineBase::Const(removed);
    }
    true
}

pub fn move_discrete_keyframe<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    old_time: Time,
    time: Time,
) -> bool {
    move_discrete_keyframes(value, &[(old_time, time)])
}

pub fn move_discrete_keyframes<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    moves: &[(Time, Time)],
) -> bool {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let mut moved = Vec::with_capacity(moves.len());
    for (index, &(old_time, _)) in moves.iter().enumerate() {
        if moves[..index]
            .iter()
            .any(|(previous, _)| previous.approx_eq(old_time))
        {
            return false;
        }
        let Some(keyframe) = keyframes
            .iter()
            .find(|keyframe| keyframe.time().approx_eq(old_time))
            .cloned()
        else {
            return false;
        };
        moved.push(keyframe);
    }
    keyframes.retain(|keyframe| {
        !moves
            .iter()
            .any(|(old_time, _)| keyframe.time().approx_eq(*old_time))
    });
    let mut destinations = Vec::with_capacity(moved.len());
    for (mut keyframe, &(_, time)) in moved.into_iter().zip(moves) {
        keyframes.retain(|other| !other.time().approx_eq(time));
        destinations.retain(|other: &T::Keyframe| !other.time().approx_eq(time));
        *keyframe.time_mut() = time;
        destinations.push(keyframe);
    }
    keyframes.extend(destinations);
    keyframes.sort_by_key(TimelineKeyframe::time);
    true
}

/// Updates selected values before moving their keys as a single operation.
/// Work on a copy so stale/duplicate source times cannot partially edit a timeline.
pub fn update_keyframes<T: TimelineValueType>(
    value: &mut TimelineValue<T>,
    changes: &[(Time, Time, T)],
) -> bool {
    if changes.is_empty() {
        return false;
    }
    let mut next = value.clone();
    let TimelineBase::Keyframes(keyframes) = &mut next.base else {
        return false;
    };
    for (old_time, _, stored) in changes {
        let Some(keyframe) = keyframes
            .iter_mut()
            .find(|keyframe| keyframe.time().approx_eq(*old_time))
        else {
            return false;
        };
        *keyframe.value_mut() = stored.clone();
    }
    let moves = changes
        .iter()
        .map(|(old, time, _)| (*old, *time))
        .collect::<Vec<_>>();
    if !move_discrete_keyframes(&mut next, &moves) {
        return false;
    }
    *value = next;
    true
}

pub fn set_json_discrete_value(
    value: &mut serde_json::Value,
    time: Time,
    next: serde_json::Value,
    frame_step: Time,
) -> Result<bool, String> {
    let keyframes = json_keyframes_mut(value)?;
    for keyframe in keyframes.iter() {
        json_keyframe_time(keyframe)?;
    }
    if let Some(keyframe) = keyframes.iter_mut().find(|keyframe| {
        same_frame(
            json_keyframe_time(keyframe)
                .expect("validated discrete keyframe time must remain valid"),
            time,
            frame_step,
        )
    }) {
        let keyframe = keyframe
            .as_object_mut()
            .ok_or_else(|| "discrete keyframe must be an object".to_string())?;
        if !keyframe.contains_key("value") {
            return Err("discrete keyframe value is missing".to_string());
        }
        keyframe.insert("value".to_string(), next);
        keyframe.insert(
            "time".to_string(),
            serde_json::to_value(time).expect("keyframe time must serialize"),
        );
        sort_json_keyframes(keyframes)?;
        return Ok(true);
    }
    keyframes.push(serde_json::json!({
        "id": Uuid::new_v4(),
        "time": time,
        "value": next,
    }));
    sort_json_keyframes(keyframes)?;
    Ok(true)
}

pub fn add_json_discrete_keyframe(
    value: &mut serde_json::Value,
    time: Time,
    frame_step: Time,
) -> Result<bool, String> {
    let current = {
        let keyframes = json_keyframes(value)?;
        for keyframe in keyframes {
            json_keyframe_time(keyframe)?;
        }
        keyframes
            .iter()
            .rev()
            .find(|keyframe| {
                json_keyframe_time(keyframe)
                    .expect("validated discrete keyframe time must remain valid")
                    <= time
            })
            .or_else(|| keyframes.first())
            .and_then(|keyframe| keyframe.get("value"))
            .cloned()
            .ok_or_else(|| "discrete timeline has no keyframe value".to_string())?
    };
    set_json_discrete_value(value, time, current, frame_step)
}

pub fn delete_json_discrete_keyframe(
    value: &mut serde_json::Value,
    time: Time,
    frame_step: Time,
) -> Result<bool, String> {
    // A batch may already have removed the last key before encountering a stale
    // selection entry. Match the typed deletion helpers' constant-timeline no-op.
    if value
        .get("base")
        .and_then(|base| base.get("const"))
        .is_some()
    {
        return Ok(false);
    }
    let (removed, empty) = {
        let keyframes = json_keyframes_mut(value)?;
        for keyframe in keyframes.iter() {
            json_keyframe_time(keyframe)?;
        }
        let Some(index) = keyframes.iter().position(|keyframe| {
            same_frame(
                json_keyframe_time(keyframe)
                    .expect("validated discrete keyframe time must remain valid"),
                time,
                frame_step,
            )
        }) else {
            return Ok(false);
        };
        let removed = keyframes
            .remove(index)
            .get("value")
            .cloned()
            .ok_or_else(|| "discrete keyframe value is missing".to_string())?;
        (removed, keyframes.is_empty())
    };
    if empty {
        *value
            .get_mut("base")
            .ok_or_else(|| "discrete timeline base is missing".to_string())? =
            serde_json::json!({ "const": removed });
    }
    Ok(true)
}

pub fn move_json_discrete_keyframe(
    value: &mut serde_json::Value,
    old_time: Time,
    time: Time,
) -> Result<bool, String> {
    move_json_discrete_keyframes(value, &[(old_time, time)])
}

pub fn move_json_discrete_keyframes(
    value: &mut serde_json::Value,
    moves: &[(Time, Time)],
) -> Result<bool, String> {
    let keyframes = json_keyframes_mut(value)?;
    for keyframe in keyframes.iter() {
        json_keyframe_time(keyframe)?;
    }
    let mut moved = Vec::with_capacity(moves.len());
    for (index, &(old_time, _)) in moves.iter().enumerate() {
        if moves[..index]
            .iter()
            .any(|(previous, _)| previous.approx_eq(old_time))
        {
            return Ok(false);
        }
        let Some(keyframe) = keyframes.iter().find(|keyframe| {
            json_keyframe_time(keyframe)
                .expect("validated discrete keyframe time must remain valid")
                .approx_eq(old_time)
        }) else {
            return Ok(false);
        };
        moved.push(keyframe.clone());
    }
    keyframes.retain(|keyframe| {
        !moves.iter().any(|(old_time, _)| {
            json_keyframe_time(keyframe)
                .expect("validated discrete keyframe time must remain valid")
                .approx_eq(*old_time)
        })
    });
    let mut destinations = Vec::with_capacity(moved.len());
    for (mut keyframe, &(_, time)) in moved.into_iter().zip(moves) {
        keyframes.retain(|keyframe| {
            !json_keyframe_time(keyframe)
                .expect("validated discrete keyframe time must remain valid")
                .approx_eq(time)
        });
        destinations.retain(|keyframe| {
            !json_keyframe_time(keyframe)
                .expect("validated discrete keyframe time must remain valid")
                .approx_eq(time)
        });
        keyframe
            .as_object_mut()
            .expect("validated discrete keyframe must remain an object")
            .insert(
                "time".to_string(),
                serde_json::to_value(time).expect("keyframe time must serialize"),
            );
        destinations.push(keyframe);
    }
    keyframes.extend(destinations);
    sort_json_keyframes(keyframes)?;
    Ok(true)
}

pub fn copy_json_discrete_keyframes(
    value: &serde_json::Value,
    selected: &[Time],
    timeline_type: &str,
) -> Result<Option<KeyframeClipboard>, String> {
    let mut copied = Vec::new();
    for keyframe in json_keyframes(value)? {
        if selected.contains(&json_keyframe_time(keyframe)?) {
            copied.push(keyframe.clone());
        }
    }
    if copied.is_empty() {
        return Ok(None);
    }
    let times = copied
        .iter()
        .map(json_keyframe_time)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(KeyframeClipboard {
        len: copied.len(),
        times,
        values: Rc::new(JsonDiscreteClipboard {
            timeline_type: timeline_type.to_string(),
            keyframes: copied,
        }),
    }))
}

pub fn paste_json_discrete_keyframes(
    value: &mut serde_json::Value,
    clipboard: &KeyframeClipboard,
    times: &[Time],
    timeline_type: &str,
) -> Result<Option<Vec<Time>>, String> {
    let Some(source) = clipboard.values.downcast_ref::<JsonDiscreteClipboard>() else {
        return Ok(None);
    };
    if source.timeline_type != timeline_type || source.keyframes.len() != times.len() {
        return Ok(None);
    }
    let mut pasted = source.keyframes.clone();
    for (keyframe, time) in pasted.iter_mut().zip(times) {
        let keyframe = keyframe
            .as_object_mut()
            .ok_or_else(|| "discrete keyframe must be an object".to_string())?;
        keyframe.insert(
            "id".to_string(),
            serde_json::to_value(Uuid::new_v4()).expect("keyframe ID must serialize"),
        );
        keyframe.insert(
            "time".to_string(),
            serde_json::to_value(time).expect("keyframe time must serialize"),
        );
    }
    let paste_start = *times
        .iter()
        .min()
        .ok_or_else(|| "discrete keyframe paste has no times".to_string())?;
    let paste_end = *times
        .iter()
        .max()
        .ok_or_else(|| "discrete keyframe paste has no times".to_string())?;
    let keyframes = json_keyframes_mut(value)?;
    for keyframe in keyframes.iter() {
        json_keyframe_time(keyframe)?;
    }
    keyframes.retain(|keyframe| {
        let time = json_keyframe_time(keyframe)
            .expect("validated discrete keyframe time must remain valid");
        if paste_start == paste_end {
            time != paste_start
        } else {
            time < paste_start || time > paste_end
        }
    });
    keyframes.extend(pasted);
    sort_json_keyframes(keyframes)?;
    Ok(Some(times.to_vec()))
}

fn json_keyframes(value: &serde_json::Value) -> Result<&Vec<serde_json::Value>, String> {
    value
        .get("base")
        .and_then(|base| base.get("keyframes"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "discrete timeline is not keyframed".to_string())
}

fn json_keyframes_mut(
    value: &mut serde_json::Value,
) -> Result<&mut Vec<serde_json::Value>, String> {
    value
        .get_mut("base")
        .and_then(|base| base.get_mut("keyframes"))
        .and_then(serde_json::Value::as_array_mut)
        .ok_or_else(|| "discrete timeline is not keyframed".to_string())
}

fn json_keyframe_time(value: &serde_json::Value) -> Result<Time, String> {
    value
        .get("time")
        .cloned()
        .ok_or_else(|| "discrete keyframe time is missing".to_string())
        .and_then(|time| {
            serde_json::from_value(time)
                .map_err(|error| format!("discrete keyframe time is invalid: {error}"))
        })
}

pub fn sort_json_keyframes(keyframes: &mut [serde_json::Value]) -> Result<(), String> {
    for keyframe in keyframes.iter() {
        json_keyframe_time(keyframe)?;
    }
    keyframes.sort_by_key(|keyframe| {
        json_keyframe_time(keyframe).expect("validated discrete keyframe time must remain valid")
    });
    Ok(())
}

pub fn set_scalar_value(value: &mut TimelineValue<f32>, time: Time, next: f32) -> bool {
    if !next.is_finite() {
        return false;
    }
    edit_curve_value(
        value,
        time,
        next,
        |current, next| (*current - *next).abs() <= 0.000_001,
        CurveEditPolicy {
            unchanged_keyframe_is_noop: true,
            insert: CurveKeyframeInsert::InheritPreviousInterpolation,
        },
    )
}

pub fn delete_scalar_keyframe(value: &mut TimelineValue<f32>, time: Time) -> bool {
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let Some(index) = keyframes
        .iter()
        .position(|keyframe| keyframe.time.approx_eq(time))
    else {
        return false;
    };
    keyframes.remove(index);
    if keyframes.is_empty() {
        value.base = TimelineBase::Const(0.0);
    }
    true
}

pub fn update_scalar_keyframe(
    value: &mut TimelineValue<f32>,
    old_time: Time,
    time: Time,
    next: f32,
) -> bool {
    if !next.is_finite() {
        return false;
    }
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let Some(index) = keyframes
        .iter()
        .position(|keyframe| keyframe.time.approx_eq(old_time))
    else {
        return false;
    };
    let mut keyframe = keyframes.remove(index);
    keyframes.retain(|other| !other.time.approx_eq(time));
    keyframe.time = time;
    keyframe.value = next;
    keyframes.push(keyframe);
    keyframes.sort_by_key(|keyframe| keyframe.time);
    true
}

pub fn set_interpolation<T>(
    value: &mut TimelineValue<T>,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> bool
where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
{
    let TimelineBase::Keyframes(keyframes) = &mut value.base else {
        return false;
    };
    let Some(keyframe) = keyframes
        .iter_mut()
        .find(|keyframe| keyframe.id == owner_id)
    else {
        return false;
    };
    if keyframe.interpolation_to_next == interpolation {
        return false;
    }
    keyframe.interpolation_to_next = interpolation;
    true
}

pub fn set_scalar_interpolation(
    value: &mut TimelineValue<f32>,
    owner_id: Uuid,
    interpolation: Interpolation,
) -> bool {
    set_interpolation(value, owner_id, interpolation)
}
