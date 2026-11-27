//! Common descriptions consumed by thin native widget renderers.
use serde_json::{Value, json};
use shrimply_i18n::{text, text_args};

/// Exact numeric constraints shared by native configuration rows.
pub struct NumberField {
    pub key: &'static str,
    pub label: &'static str,
    pub minimum: &'static str,
    pub maximum: &'static str,
    pub step: &'static str,
    pub digits: usize,
}

pub fn tag_editor(tags: &[String]) -> Value {
    json!({"label":text("Tags"),"placeholder":text("Add tag"),"add_label":text("Add tag"),"add_icon":"list-add-symbolic","remove_icon":"window-close-symbolic",
        "tags":tags.iter().map(|tag|json!({"value":tag,"remove_label":text_args("Remove tag %{tag}",&[("tag",tag.clone())])})).collect::<Vec<_>>()})
}

pub fn choice_label(label: &str, selected: Option<&str>) -> String {
    selected.map_or_else(
        || {
            text(if label.is_empty() {
                "Add modifier"
            } else {
                label
            })
            .into_owned()
        },
        |selected| {
            text_args(
                "%{label}: %{value}",
                &[
                    ("label", text(label).into_owned()),
                    ("value", text(selected).into_owned()),
                ],
            )
        },
    )
}

/// Supply the native widget contract and stable child identities recursively.
/// Descriptions retain explicit read-only, disabled, and localized values.
pub fn normalize_control(control: &mut Value, id: &str) {
    control["id"] = json!(id);
    for (key, value) in [
        ("subtitle", json!("")),
        ("display", json!("")),
        ("enabled", json!(true)),
        ("editable", json!(true)),
    ] {
        if control.get(key).is_none_or(Value::is_null) {
            control[key] = value;
        }
    }
    if let Some(children) = control["controls"].as_array_mut() {
        for (index, child) in children.iter_mut().enumerate() {
            normalize_control(child, &format!("{id}:{index}"));
        }
    }
}
