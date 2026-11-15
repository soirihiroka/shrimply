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

pub const RECENT_PROJECT_MENU_LABEL: &str = "Project options";

#[derive(Clone, Copy, Debug)]
pub enum RecentProjectAction {
    Info,
    ShowInFiles,
    Delete,
}

impl RecentProjectAction {
    pub const ALL: [Self; 3] = [Self::Info, Self::ShowInFiles, Self::Delete];

    pub fn service(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::ShowInFiles => "show_in_files",
            Self::Delete => "remove_recent",
        }
    }

    pub fn label(self) -> String {
        text(match self {
            Self::Info => "Info",
            Self::ShowInFiles => "Show in Files",
            Self::Delete => "Delete",
        })
        .into_owned()
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::Info => "info-outline-symbolic",
            Self::ShowInFiles => "folder-open-symbolic",
            Self::Delete => "user-trash-symbolic",
        }
    }
}

pub fn recent_project_subtitle(date: Option<&str>) -> String {
    date.map_or_else(
        || text("Last edited time unavailable").into_owned(),
        |date| text_args("Last edited %{date}", &[("date", date.to_owned())]),
    )
}

pub struct RecentProjectInfo {
    pub title: String,
    pub fields: [RecentProjectInfoField; 2],
    pub close_label: String,
}

pub struct RecentProjectInfoField {
    pub label: String,
    pub value: String,
    pub action: Option<RecentProjectAction>,
}

pub fn recent_project_info(name: &str, path: &str, date: Option<&str>) -> RecentProjectInfo {
    RecentProjectInfo {
        title: name.to_owned(),
        close_label: text("Close").into_owned(),
        fields: [
            RecentProjectInfoField {
                label: text("Last Edited").into_owned(),
                value: date.map_or_else(|| text("Unavailable").into_owned(), str::to_owned),
                action: None,
            },
            RecentProjectInfoField {
                label: text("File Location").into_owned(),
                value: path.to_owned(),
                action: Some(RecentProjectAction::ShowInFiles),
            },
        ],
    }
}
