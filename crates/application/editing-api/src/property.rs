use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use shrimply_math_core::{Time, time_from_frame};
use shrimply_project_document::project::{ItemMut, ItemRef, Project};
use uuid::Uuid;

use crate::protocol::{
    DeleteKeyframesRequest, DeletePropertyExpressionRequest, KeyframeInput, KeyframeValue,
    UpsertKeyframesRequest, UpsertPropertyExpressionRequest,
};
use crate::query::model_item_address;

#[derive(Clone, Copy, Eq, PartialEq)]
enum ValueKind {
    Scalar,
    Vec2,
}

pub fn upsert_keyframes(
    project: &mut Project,
    request: &UpsertKeyframesRequest,
) -> Result<Uuid, String> {
    if request.keyframes.is_empty() {
        return Err("upsert_keyframes requires at least one keyframe".to_string());
    }
    let address = model_item_address(&request.address)?;
    let times = local_keyframe_times(
        project,
        &address,
        request.keyframes.iter().map(|keyframe| keyframe.frame),
    )?;
    let mut keyed = request.keyframes.iter().zip(times).collect::<Vec<_>>();
    keyed.sort_by_key(|(_, time)| *time);
    if keyed.windows(2).any(|pair| pair[0].1.approx_eq(pair[1].1)) {
        return Err("upsert_keyframes contains duplicate projected frames".to_string());
    }
    let kind = value_kind(keyed[0].0.value)?;
    for (keyframe, _) in &keyed {
        if value_kind(keyframe.value)? != kind {
            return Err("all keyframes in one request must use the same value kind".to_string());
        }
    }
    mutate_item(project, &address, |value| {
        let property = timeline_value_mut(value, &request.property_path)?;
        require_kind(property, kind)?;
        let base = property
            .get_mut("base")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("{} is not a TimelineValue", request.property_path))?;
        let keyframes = if base.contains_key("const") {
            base.clear();
            base.insert("keyframes".to_string(), Value::Array(Vec::new()));
            base.get_mut("keyframes")
                .and_then(Value::as_array_mut)
                .expect("new keyframe array must exist")
        } else {
            base.get_mut("keyframes")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| {
                    format!(
                        "{} has an invalid TimelineValue base",
                        request.property_path
                    )
                })?
        };
        validate_keyframes(keyframes, kind, &request.property_path)?;
        for (input, time) in keyed {
            upsert(keyframes, input, time)?;
        }
        keyframes.sort_by_key(|keyframe| {
            serde_json::from_value::<Time>(keyframe["time"].clone())
                .expect("validated keyframe time must deserialize")
        });
        Ok(())
    })?;
    Ok(address.item_id())
}

pub fn delete_keyframes(
    project: &mut Project,
    request: &DeleteKeyframesRequest,
) -> Result<Uuid, String> {
    if request.frames.is_empty() {
        return Err("delete_keyframes requires at least one frame".to_string());
    }
    let address = model_item_address(&request.address)?;
    let times = local_keyframe_times(project, &address, request.frames.iter().copied())?;
    let mut requested = request
        .frames
        .iter()
        .copied()
        .zip(times)
        .collect::<Vec<_>>();
    requested.sort_by_key(|(_, time)| *time);
    if requested
        .windows(2)
        .any(|pair| pair[0].1.approx_eq(pair[1].1))
    {
        return Err("delete_keyframes contains duplicate projected frames".to_string());
    }
    mutate_item(project, &address, |value| {
        let property = timeline_value_mut(value, &request.property_path)?;
        let base = property
            .get_mut("base")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| format!("{} is not a TimelineValue", request.property_path))?;
        let keyframes = base
            .get_mut("keyframes")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("{} is not keyframed", request.property_path))?;
        if keyframes.is_empty() {
            return Err(format!("{} has no keyframes", request.property_path));
        }
        let kind = stored_value_kind(
            keyframes[0]
                .get("value")
                .ok_or_else(|| format!("{} has an invalid keyframe", request.property_path))?,
        )?;
        validate_keyframes(keyframes, kind, &request.property_path)?;
        let fallback = keyframes[0]["value"].clone();
        let mut found = vec![false; requested.len()];
        keyframes.retain(|keyframe| {
            let time = serde_json::from_value::<Time>(keyframe["time"].clone())
                .expect("validated keyframe time must deserialize");
            if let Some((index, _)) = requested
                .iter()
                .enumerate()
                .find(|(_, (_, requested))| time.approx_eq(*requested))
            {
                found[index] = true;
                false
            } else {
                true
            }
        });
        if let Some(index) = found.iter().position(|found| !found) {
            return Err(format!(
                "no keyframe exists at projected frame {}",
                requested[index].0
            ));
        }
        if keyframes.is_empty() {
            base.clear();
            base.insert("const".to_string(), fallback);
        }
        Ok(())
    })?;
    Ok(address.item_id())
}

pub fn upsert_expression(
    project: &mut Project,
    request: &UpsertPropertyExpressionRequest,
) -> Result<Uuid, String> {
    if request.source.trim().is_empty() {
        return Err("expression source must not be empty".to_string());
    }
    let address = model_item_address(&request.address)?;
    mutate_item(project, &address, |value| {
        let property = timeline_value_mut(value, expression_property_path(&request.property_path))?;
        require_supported_property(property)?;
        let current = property.get("expression").and_then(Value::as_object);
        let id = current
            .and_then(|expression| expression.get("id"))
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok())
            .unwrap_or_else(Uuid::new_v4);
        let enabled = request.enabled.unwrap_or_else(|| {
            current
                .and_then(|expression| expression.get("enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(true)
        });
        property.insert(
            "expression".to_string(),
            json!({"id": id, "enabled": enabled, "source": request.source}),
        );
        Ok(())
    })?;
    Ok(address.item_id())
}

pub fn delete_expression(
    project: &mut Project,
    request: &DeletePropertyExpressionRequest,
) -> Result<Uuid, String> {
    let address = model_item_address(&request.address)?;
    mutate_item(project, &address, |value| {
        let property = timeline_value_mut(value, expression_property_path(&request.property_path))?;
        require_supported_property(property)?;
        if property
            .get("expression")
            .is_none_or(|expression| expression.is_null())
        {
            return Err(format!(
                "{} does not have an expression",
                request.property_path
            ));
        }
        property.insert("expression".to_string(), Value::Null);
        Ok(())
    })?;
    Ok(address.item_id())
}

fn local_keyframe_times(
    project: &Project,
    address: &shrimply_project_document::project::ItemAddress,
    frames: impl Iterator<Item = u64>,
) -> Result<Vec<Time>, String> {
    let (start, end) = match project
        .item(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemRef::Comment(_) => return Err("comment clips do not support keyframes".to_string()),
        ItemRef::Caption(_) => return Err("caption clips do not support keyframes".to_string()),
        ItemRef::Video(item) => (item.start, item.end),
        ItemRef::Audio(item) => (item.start, item.end),
    };
    frames
        .map(|frame| {
            let timeline_time = time_from_frame(frame, project.fps)
                .ok_or_else(|| "frame exceeds the supported exact fraction range".to_string())?;
            let sequence_time = project
                .timeline_time_to_sequence(&address.track(), timeline_time)
                .ok_or_else(|| "clip scope does not resolve in the project".to_string())?;
            if sequence_time < start || sequence_time > end {
                return Err(format!(
                    "projected frame {frame} is outside the addressed clip"
                ));
            }
            project
                .keyframe_time(address, timeline_time)
                .ok_or_else(|| "could not resolve the clip's local keyframe time".to_string())
        })
        .collect()
}

pub(crate) fn mutate_item(
    project: &mut Project,
    address: &shrimply_project_document::project::ItemAddress,
    update: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    match project
        .item_mut(address)
        .ok_or_else(|| "clip was not found".to_string())?
    {
        ItemMut::Comment(_) => Err("comment clips do not contain timeline properties".to_string()),
        ItemMut::Caption(_) => Err("caption clips do not contain timeline properties".to_string()),
        ItemMut::Video(item) => mutate_serialized(item, update),
        ItemMut::Audio(item) => mutate_serialized(item, update),
    }
}

fn mutate_serialized<T: Serialize + DeserializeOwned>(
    item: &mut T,
    update: impl FnOnce(&mut Value) -> Result<(), String>,
) -> Result<(), String> {
    let mut value = serde_json::to_value(&*item)
        .map_err(|error| format!("could not inspect clip properties: {error}"))?;
    update(&mut value)?;
    *item = serde_json::from_value(value)
        .map_err(|error| format!("could not apply clip property edit: {error}"))?;
    Ok(())
}

fn timeline_value_mut<'a>(
    value: &'a mut Value,
    property_path: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    if !property_path.starts_with('/') {
        return Err("property_path must be a JSON Pointer beginning with /".to_string());
    }
    let property = value
        .pointer_mut(property_path)
        .ok_or_else(|| format!("property path {property_path} was not found"))?;
    let object = property
        .as_object_mut()
        .ok_or_else(|| format!("{property_path} is not a TimelineValue"))?;
    if !object.get("id").is_some_and(Value::is_string) || !object.contains_key("base") {
        return Err(format!("{property_path} is not a TimelineValue"));
    }
    Ok(object)
}

fn expression_property_path(path: &str) -> &str {
    path.strip_suffix("/expression").unwrap_or(path)
}

fn require_supported_property(property: &Map<String, Value>) -> Result<(), String> {
    let base = property
        .get("base")
        .and_then(Value::as_object)
        .ok_or_else(|| "property has an invalid TimelineValue base".to_string())?;
    if let Some(value) = base.get("const") {
        stored_value_kind(value)?;
        return Ok(());
    }
    let keyframes = base
        .get("keyframes")
        .and_then(Value::as_array)
        .ok_or_else(|| "property has an invalid TimelineValue base".to_string())?;
    if let Some(value) = keyframes.first().and_then(|keyframe| keyframe.get("value")) {
        stored_value_kind(value)?;
    }
    Ok(())
}

fn require_kind(property: &Map<String, Value>, expected: ValueKind) -> Result<(), String> {
    let base = property
        .get("base")
        .and_then(Value::as_object)
        .ok_or_else(|| "property has an invalid TimelineValue base".to_string())?;
    let stored = base.get("const").or_else(|| {
        base.get("keyframes")
            .and_then(Value::as_array)
            .and_then(|keyframes| keyframes.first())
            .and_then(|keyframe| keyframe.get("value"))
    });
    if stored.is_some_and(|value| stored_value_kind(value) != Ok(expected)) {
        return Err("keyframe value kind does not match the addressed property".to_string());
    }
    Ok(())
}

fn validate_keyframes(
    keyframes: &[Value],
    kind: ValueKind,
    property_path: &str,
) -> Result<(), String> {
    for keyframe in keyframes {
        let object = keyframe
            .as_object()
            .ok_or_else(|| format!("{property_path} has an invalid keyframe"))?;
        serde_json::from_value::<Time>(
            object
                .get("time")
                .cloned()
                .ok_or_else(|| format!("{property_path} has an invalid keyframe time"))?,
        )
        .map_err(|error| format!("{property_path} has an invalid keyframe time: {error}"))?;
        if object
            .get("value")
            .is_none_or(|value| stored_value_kind(value) != Ok(kind))
        {
            return Err(format!(
                "{property_path} contains a different keyframe value kind"
            ));
        }
    }
    Ok(())
}

fn upsert(keyframes: &mut Vec<Value>, input: &KeyframeInput, time: Time) -> Result<(), String> {
    let value = keyframe_value(input.value)?;
    let interpolation = serde_json::to_value(input.interpolation)
        .map_err(|error| format!("could not encode keyframe interpolation: {error}"))?;
    let existing = keyframes.iter_mut().find(|keyframe| {
        keyframe
            .get("time")
            .cloned()
            .and_then(|time| serde_json::from_value::<Time>(time).ok())
            .is_some_and(|current| current.approx_eq(time))
    });
    if let Some(existing) = existing {
        let id = existing
            .get("id")
            .cloned()
            .unwrap_or_else(|| Value::String(Uuid::new_v4().to_string()));
        *existing = json!({
            "id": id,
            "time": time,
            "value": value,
            "interpolation_to_next": interpolation,
        });
    } else {
        keyframes.push(json!({
            "id": Uuid::new_v4(),
            "time": time,
            "value": value,
            "interpolation_to_next": interpolation,
        }));
    }
    Ok(())
}

fn value_kind(value: KeyframeValue) -> Result<ValueKind, String> {
    keyframe_value(value).map(|_| match value {
        KeyframeValue::Scalar(_) => ValueKind::Scalar,
        KeyframeValue::Vec2(_) => ValueKind::Vec2,
    })
}

fn keyframe_value(value: KeyframeValue) -> Result<Value, String> {
    match value {
        KeyframeValue::Scalar(value) if value.is_finite() => Ok(json!(value)),
        KeyframeValue::Vec2(value) if value.x.is_finite() && value.y.is_finite() => {
            Ok(json!([value.x, value.y]))
        }
        KeyframeValue::Scalar(_) | KeyframeValue::Vec2(_) => {
            Err("keyframe values must be finite".to_string())
        }
    }
}

fn stored_value_kind(value: &Value) -> Result<ValueKind, String> {
    if value.is_number() {
        return Ok(ValueKind::Scalar);
    }
    if value
        .as_array()
        .is_some_and(|values| values.len() == 2 && values.iter().all(Value::is_number))
    {
        return Ok(ValueKind::Vec2);
    }
    Err("only scalar and vec2 timeline properties are supported".to_string())
}
