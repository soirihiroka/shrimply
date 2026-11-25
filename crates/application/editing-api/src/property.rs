use glam::Vec2;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shrimply_math_core::{Time, time_from_frame};
use shrimply_project_document::project::{ItemAddress, ItemKind, Project};
use shrimply_property_model::timeline_value::{
    CurveEditPolicy, CurveKeyframeInsert, TimelineBase, TimelineCurveKeyframe, TimelineExpression,
    TimelineValue, TimelineValueType, edit_curve_value, remove_keyframes,
};
use uuid::Uuid;

use crate::protocol::{
    DeleteKeyframesRequest, DeletePropertyExpressionRequest, KeyframeInput, KeyframeValue,
    UpsertKeyframesRequest, UpsertPropertyExpressionRequest,
};

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum NumericProperty {
    Integer(TimelineValue<i64>),
    Scalar(TimelineValue<f32>),
    Vector(TimelineValue<Vec2>),
}

pub fn upsert_keyframes(
    project: &mut Project,
    request: &UpsertKeyframesRequest,
) -> Result<Uuid, String> {
    let address = &request.address;
    let first = request
        .keyframes
        .first()
        .ok_or("provide at least one keyframe")?;
    let frames = keyframe_times(
        project,
        address,
        request.keyframes.iter().map(|keyframe| keyframe.frame),
    )?;
    let inputs = request
        .keyframes
        .iter()
        .zip(frames.into_iter().map(|(_, time)| time));
    edit_property(project, address, &request.property_path, |property| {
        let value = match first.value {
            KeyframeValue::Scalar(_) => upsert(property.take(), inputs, |value| match value {
                KeyframeValue::Scalar(value) if value.is_finite() => Some(value),
                _ => None,
            })
            .map(NumericProperty::Scalar),
            KeyframeValue::Vec2(_) => upsert(property.take(), inputs, |value| match value {
                KeyframeValue::Vec2(value) if value.x.is_finite() && value.y.is_finite() => {
                    Some(Vec2::new(value.x, value.y))
                }
                _ => None,
            })
            .map(NumericProperty::Vector),
        }?;
        *property = serde_json::to_value(value).map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub fn delete_keyframes(
    project: &mut Project,
    request: &DeleteKeyframesRequest,
) -> Result<Uuid, String> {
    let address = &request.address;
    let frames = keyframe_times(project, address, request.frames.iter().copied())?;
    edit_property(project, address, &request.property_path, |property| {
        let mut value: NumericProperty = serde_json::from_value(property.take())
            .map_err(|_| "only numeric scalar and vec2 keyframes are supported")?;
        match &mut value {
            NumericProperty::Integer(value) => delete(value, &frames)?,
            NumericProperty::Scalar(value) => delete(value, &frames)?,
            NumericProperty::Vector(value) => delete(value, &frames)?,
        }
        *property = serde_json::to_value(value).map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub fn upsert_expression(
    project: &mut Project,
    request: &UpsertPropertyExpressionRequest,
) -> Result<Uuid, String> {
    if request
        .source
        .as_ref()
        .is_some_and(|source| source.trim().is_empty())
        || (request.source.is_none() && request.enabled.is_none())
    {
        return Err("provide a nonempty source or an enabled state".into());
    }
    let path = request
        .property_path
        .strip_suffix("/expression")
        .unwrap_or(&request.property_path);
    edit_property(project, &request.address, path, |property| {
        let slot = &mut property["expression"];
        let current: Option<TimelineExpression> =
            serde_json::from_value(slot.take()).map_err(|error| error.to_string())?;
        let expression = if let Some(mut expression) = current {
            expression.update(request.source.clone(), request.enabled);
            expression
        } else {
            TimelineExpression {
                id: Uuid::new_v4(),
                source: request
                    .source
                    .clone()
                    .ok_or("a new expression requires source")?,
                enabled: request.enabled.unwrap_or(true),
            }
        };
        *slot = serde_json::to_value(expression).map_err(|error| error.to_string())?;
        Ok(())
    })
}

pub fn delete_expression(
    project: &mut Project,
    request: &DeletePropertyExpressionRequest,
) -> Result<Uuid, String> {
    let path = request
        .property_path
        .strip_suffix("/expression")
        .unwrap_or(&request.property_path);
    edit_property(project, &request.address, path, |property| {
        if property["expression"].take().is_null() {
            return Err("property has no expression".into());
        }
        Ok(())
    })
}

fn keyframe_times(
    project: &Project,
    address: &ItemAddress,
    frames: impl Iterator<Item = u64>,
) -> Result<Vec<(u64, Time)>, String> {
    if !matches!(address.kind(), ItemKind::Video | ItemKind::Audio) {
        return Err("this clip does not support keyframes".into());
    }
    let (start, end) = project.item(address).ok_or("clip was not found")?.times();
    let track = address.track();
    let times = frames
        .map(|frame| {
            let timeline_time = time_from_frame(frame, project.fps)
                .ok_or("frame is outside the supported range")?;
            let sequence_time = project
                .timeline_time_to_sequence(&track, timeline_time)
                .ok_or("clip scope does not resolve in the project")?;
            if sequence_time < start || sequence_time > end {
                return Err(format!(
                    "projected frame {frame} is outside the addressed clip"
                ));
            }
            let local = project
                .keyframe_time(address, timeline_time)
                .ok_or("clip keyframe time could not be resolved")?;
            Ok((frame, local))
        })
        .collect::<Result<Vec<_>, String>>()?;
    if times.is_empty() {
        return Err("provide at least one keyframe".into());
    }
    let mut sorted = times.iter().map(|(_, time)| *time).collect::<Vec<_>>();
    sorted.sort_unstable();
    if sorted.windows(2).any(|pair| pair[0].approx_eq(pair[1])) {
        return Err("request contains duplicate projected frames".into());
    }
    Ok(times)
}

fn upsert<'a, T>(
    property: Value,
    inputs: impl Iterator<Item = (&'a KeyframeInput, Time)>,
    parse: impl Fn(KeyframeValue) -> Option<T>,
) -> Result<TimelineValue<T>, String>
where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
    TimelineValue<T>: serde::de::DeserializeOwned,
{
    let mismatch = "keyframe values must be finite and match the addressed property type";
    let mut value: TimelineValue<T> = serde_json::from_value(property).map_err(|_| mismatch)?;
    if matches!(value.base, TimelineBase::Const(_)) {
        value.base = TimelineBase::Keyframes(Vec::new());
    }
    for (input, time) in inputs {
        edit_curve_value(
            &mut value,
            time,
            parse(input.value).ok_or(mismatch)?,
            |_, _| false,
            CurveEditPolicy {
                unchanged_keyframe_is_noop: false,
                insert: CurveKeyframeInsert::Default,
            },
        );
        if let TimelineBase::Keyframes(keyframes) = &mut value.base {
            keyframes
                .iter_mut()
                .find(|keyframe| keyframe.time == time)
                .expect("edited keyframe must exist")
                .interpolation_to_next = input.interpolation;
        }
    }
    Ok(value)
}

fn delete<T>(value: &mut TimelineValue<T>, frames: &[(u64, Time)]) -> Result<(), String>
where
    T: TimelineValueType<Keyframe = TimelineCurveKeyframe<T>>,
{
    let TimelineBase::Keyframes(keyframes) = &value.base else {
        return Err("property is not keyframed".into());
    };
    let fallback = keyframes
        .first()
        .ok_or("property has no keyframes")?
        .value
        .clone();
    for (frame, time) in frames {
        if !keyframes
            .iter()
            .any(|keyframe| keyframe.time.approx_eq(*time))
        {
            return Err(format!("no keyframe exists at projected frame {frame}"));
        }
    }
    remove_keyframes(
        value,
        |keyframe| {
            frames
                .iter()
                .any(|(_, time)| keyframe.time.approx_eq(*time))
        },
        fallback,
    );
    Ok(())
}

fn edit_property(
    project: &mut Project,
    address: &ItemAddress,
    path: &str,
    edit: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<Uuid, String> {
    if !path.starts_with('/') {
        return Err("property_path must be a JSON Pointer beginning with /".into());
    }
    if !matches!(address.kind(), ItemKind::Video | ItemKind::Audio) {
        return Err("this clip does not contain timeline properties".into());
    }
    let mut item = serde_json::to_value(project.item(address).ok_or("clip was not found")?)
        .map_err(|error| error.to_string())?;
    let property = item
        .pointer_mut(path)
        .filter(|property| {
            property.get("id").is_some_and(Value::is_string)
                && property.get("base").is_some_and(Value::is_object)
        })
        .ok_or_else(|| format!("timeline property {path} was not found"))?;
    edit(property)?;
    project
        .item_mut(address)
        .expect("serialized clip must still exist")
        .replace_from_json(item)
        .map_err(|error| error.to_string())?;
    Ok(address.item_id())
}
