use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorSection, ProjectPresentation,
};

use super::{CategoryIcon, InspectorCategory, InspectorListItem, detail_item};

pub(super) fn categories(project: &ProjectPresentation) -> Vec<InspectorCategory> {
    let mut settings = InspectorControl::new(ControlKind::ProjectSettings, "", "Project Settings")
        .components(vec![
            project.canvas_size.width.to_string(),
            project.canvas_size.height.to_string(),
            shrimply_math_core::fraction_numerator(project.frame_rate).to_string(),
            shrimply_math_core::fraction_denominator(project.frame_rate).to_string(),
        ]);
    settings.project_settings = Some(Box::new(
        shrimply_inspector_core::project::settings_presentation(
            project.canvas_size,
            project.frame_rate,
            false,
        ),
    ));
    let config = InspectorSection {
        controls: vec![
            InspectorControl::new(ControlKind::Text, "/name", "Name").value(&project.name),
            InspectorControl::new(ControlKind::MultilineText, "/tags", "Tags")
                .value(project.tags.join("\n"))
                .choices(project.tags.clone(), project.tags.clone()),
            settings,
        ],
    };
    vec![
        InspectorCategory {
            key: "project",
            label: "Project",
            icon: CategoryIcon::Project,
            items: vec![InspectorListItem::Flat(config)],
        },
        InspectorCategory {
            key: "info",
            label: "Info",
            icon: CategoryIcon::Info,
            items: vec![detail_item(&project.details())],
        },
        InspectorCategory {
            key: "performance",
            label: "Performance",
            icon: CategoryIcon::Performance,
            items: vec![InspectorListItem::Flat(
                shrimply_inspector_core::benchmarking::section(),
            )],
        },
    ]
}
