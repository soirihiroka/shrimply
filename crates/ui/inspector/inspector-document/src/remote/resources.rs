//! Native delivery only receives resources advertised by the current inspector.
use serde_json::{Value, json};
use shrimply_inspector_core::{ControlKind, InspectorController};

use crate::InspectorDocument;

pub fn resource_request(
    controller: &InspectorController,
    document: &InspectorDocument,
    request: &Value,
) -> Result<Value, String> {
    if controller.target() != document.target {
        return Err("Inspector selection changed".into());
    }
    let control = super::resolve_control(document, request)?;
    match (control.kind, request["action"].as_str()) {
        (ControlKind::FileLocation, Some("show_in_files")) if !control.value.is_empty() => {
            Ok(json!({"show_in_files":control.value}))
        }
        _ => Err("Inspector resource is unavailable".into()),
    }
}
