use serde_json::Value;
use shrimply_project_document::project::Project;
use uuid::Uuid;

use crate::protocol::{ClipAddress, ExpressionSummary, SetExpressionRequest};
use crate::query::model_item_address;

pub fn summaries(value: &Value, address: &ClipAddress) -> Vec<ExpressionSummary> {
    let mut output = Vec::new();
    collect(value, address, "", &mut output);
    output
}

pub fn set(project: &mut Project, request: &SetExpressionRequest) -> Result<Uuid, String> {
    if request.source.is_none() && request.enabled.is_none() {
        return Err("set_expression requires source or enabled".to_string());
    }
    let expression_id = Uuid::parse_str(&request.expression_id)
        .map_err(|error| format!("expression_id is not a UUID: {error}"))?;
    let address = model_item_address(&request.address)?;
    crate::property::mutate_item(project, &address, |value| {
        if update(value, expression_id, request) {
            Ok(())
        } else {
            Err(format!(
                "expression {expression_id} was not found in clip {}",
                request.address.item_id
            ))
        }
    })?;
    Ok(address.item_id())
}

fn collect(value: &Value, address: &ClipAddress, path: &str, output: &mut Vec<ExpressionSummary>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let child_path = format!("{path}/{}", json_pointer_segment(key));
                if key == "expression"
                    && let Some(expression) = summary(child, address, path.to_string())
                {
                    output.push(expression);
                } else {
                    collect(child, address, &child_path, output);
                }
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect(child, address, &format!("{path}/{index}"), output);
            }
        }
        _ => {}
    }
}

fn summary(
    value: &Value,
    address: &ClipAddress,
    property_path: String,
) -> Option<ExpressionSummary> {
    let object = value.as_object()?;
    let expression_id = object.get("id")?.as_str()?;
    Uuid::parse_str(expression_id).ok()?;
    Some(ExpressionSummary {
        address: address.clone(),
        expression_id: expression_id.to_string(),
        property_path,
        enabled: object.get("enabled")?.as_bool()?,
        source: object.get("source")?.as_str()?.to_string(),
    })
}

fn update(value: &mut Value, expression_id: Uuid, request: &SetExpressionRequest) -> bool {
    match value {
        Value::Object(object) => {
            if object
                .get_mut("expression")
                .is_some_and(|expression| update_expression(expression, expression_id, request))
            {
                return true;
            }
            object
                .iter_mut()
                .filter(|(key, _)| key.as_str() != "expression")
                .any(|(_, child)| update(child, expression_id, request))
        }
        Value::Array(values) => values
            .iter_mut()
            .any(|child| update(child, expression_id, request)),
        _ => false,
    }
}

fn update_expression(
    value: &mut Value,
    expression_id: Uuid,
    request: &SetExpressionRequest,
) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    if !object
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| Uuid::parse_str(id) == Ok(expression_id))
        || !object.get("enabled").is_some_and(Value::is_boolean)
        || !object.get("source").is_some_and(Value::is_string)
    {
        return false;
    }
    if let Some(source) = &request.source {
        object.insert("source".to_string(), Value::String(source.clone()));
    }
    if let Some(enabled) = request.enabled {
        object.insert("enabled".to_string(), Value::Bool(enabled));
    }
    true
}

fn json_pointer_segment(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
