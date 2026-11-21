use super::*;
use shrimply_project_document::project::{TagEdit, edit_tags};

pub(super) fn presentation(control: &InspectorControl, item: &str, id: &str) -> Value {
    let mut model = shrimply_components_core::presentation::tag_editor(&control.values);
    let command =
        |action: &str| json!({"item":item,"control":id,"action":action,"expected":control.values});
    model["id"] = json!(id);
    model["widget"] = json!("tags");
    model["subtitle"] = json!("");
    model["enabled"] = json!(control.sensitive);
    model["editable"] = json!(control.editable);
    model["add_command"] = command("tag_add");
    for tag in model["tags"].as_array_mut().expect("tag entries") {
        let mut remove = command("tag_remove");
        remove["value"] = tag["value"].clone();
        tag["command"] = remove;
    }
    model
}

pub(super) fn apply(
    controller: &InspectorController,
    document: &InspectorDocument,
    control: &InspectorControl,
    action: &str,
    request: &Value,
) -> Result<(), String> {
    if document.target != shrimply_inspector_core::InspectorTarget::Project
        || !control.sensitive
        || !control.editable
    {
        return Err("Project tags are unavailable".into());
    }
    if request["expected"] != json!(control.values) {
        return Err("Project tags changed; refresh the inspector".into());
    }
    let value = request["value"].as_str().ok_or("Tag text is required")?;
    let edit = match action {
        "tag_add" => TagEdit::Add(value),
        "tag_remove" => TagEdit::Remove(value),
        _ => return Err("Unknown tag action".into()),
    };
    controller.set_project_tags(&edit_tags(&control.values, edit));
    Ok(())
}
