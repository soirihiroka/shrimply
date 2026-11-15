//! Native delivery only receives resources advertised by the current inspector.
use base64::Engine;
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
        (ControlKind::InfoArtwork, Some("image")) => {
            if request["version"].as_str()
                != Some(
                    control
                        .image_version
                        .as_deref()
                        .expect("artwork control requires a content version"),
                )
            {
                return Err("Inspector artwork changed".into());
            }
            let bytes = control
                .image_bytes
                .as_deref()
                .ok_or("Inspector artwork is unavailable")?;
            Ok(json!({"image":base64::engine::general_purpose::STANDARD.encode(bytes)}))
        }
        _ => Err("Inspector resource is unavailable".into()),
    }
}
