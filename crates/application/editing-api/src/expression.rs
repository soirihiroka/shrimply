use serde::Deserialize;
use serde_json::Value;
use shrimply_property_model::timeline_value::TimelineExpression;

use crate::protocol::{ClipAddress, ExpressionSummary};

pub fn summaries(value: &Value, address: &ClipAddress) -> Vec<ExpressionSummary> {
    let mut expressions = Vec::new();
    collect(value, address, "", &mut expressions);
    expressions
}

fn collect(value: &Value, address: &ClipAddress, path: &str, output: &mut Vec<ExpressionSummary>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "expression"
                    && child.get("id").is_some()
                    && let Ok(expression) = TimelineExpression::deserialize(child)
                {
                    output.push(ExpressionSummary {
                        address: address.clone(),
                        expression_id: expression.id,
                        property_path: path.to_owned(),
                        source: expression.source,
                        enabled: expression.enabled,
                    });
                } else {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    collect(child, address, &format!("{path}/{escaped}"), output);
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
