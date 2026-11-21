use shrimply_project_document::project::ItemKind;

use shrimply_inspector_core::{ControlKind, InspectorControl, InspectorSection, TrackPresentation};

use super::{CategoryIcon, InspectorCategory, InspectorListItem, detail_item};

pub(super) fn categories(track: &TrackPresentation) -> Vec<InspectorCategory> {
    let mut controls = vec![
        InspectorControl::new(ControlKind::Boolean, "/enabled", "Enabled")
            .value(track.enabled.to_string()),
    ];
    if track.kind == ItemKind::Caption {
        controls.push(shrimply_inspector_core::selector::optional_selector(
            "/language",
            "Language",
            track.language.as_deref(),
            shrimply_project_document::project::caption_languages()
                .iter()
                .cloned()
                .map(|language| (language.clone(), language)),
        ));
    }
    vec![
        InspectorCategory {
            key: "track",
            label: "Track",
            icon: CategoryIcon::Track,
            items: vec![InspectorListItem::Flat(InspectorSection { controls })],
        },
        InspectorCategory {
            key: "info",
            label: "Info",
            icon: CategoryIcon::Info,
            items: vec![detail_item(&track.details())],
        },
    ]
}
