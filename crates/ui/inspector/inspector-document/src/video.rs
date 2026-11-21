use shrimply_inspector_core::{InspectorDetail, VideoPresentation};

use super::{
    BasicInspectorAction, CategoryIcon, InspectorCategory, InspectorItem, InspectorListItem,
    detail_item,
};

pub(super) fn categories(
    video: &VideoPresentation,
    details: &[InspectorDetail],
) -> Vec<InspectorCategory> {
    let manim = video.manim.as_ref().into_iter().flat_map(manim_items);
    vec![
        InspectorCategory {
            key: "visual",
            label: "Visual",
            icon: CategoryIcon::Visual,
            items: manim
                .chain(
                    video
                        .visual
                        .iter()
                        .cloned()
                        .map(item)
                        .chain(video.modifiers.iter().map(super::modifiers::visual))
                        .chain(std::iter::once(super::modifiers::menu(
                            shrimply_inspector_core::ControlKind::VisualModifierMenu,
                            video.modifier_choices.iter().map(|c| {
                                (c.key.clone(), c.label.to_string(), c.search_text.clone())
                            }),
                        ))),
                )
                .collect(),
        },
        InspectorCategory {
            key: "playback",
            label: "Playback",
            icon: CategoryIcon::Playback,
            items: video.playback.iter().cloned().map(item).collect(),
        },
        InspectorCategory {
            key: "info",
            label: "Info",
            icon: CategoryIcon::Info,
            items: vec![detail_item(details)],
        },
    ]
}

fn item(card: shrimply_inspector_core::VideoCard) -> InspectorListItem {
    inspector_item(card).boxed()
}

pub fn inspector_item(card: shrimply_inspector_core::VideoCard) -> InspectorItem {
    let mut item = InspectorItem::new(card.key, card.title, card.section);
    item.actions = card
        .actions
        .into_iter()
        .map(|action| shrimply_inspector_core::item::HeaderAction {
            icon: action.icon,
            tooltip: action.tooltip,
            sensitive: action.sensitive,
            activate: BasicInspectorAction::Video(action.activate),
        })
        .collect();
    if let Some(mask) = &card.alpha_mask {
        item = item.alpha_mask(
            shrimply_project_document::project::VisualAlphaMaskTarget::Compositing,
            mask,
        );
    }
    if let Some(reset) = card.reset {
        item = item.reset(BasicInspectorAction::ResetVideo(reset));
    }
    if let Some(facet) = card.preview_facet {
        item = item.preview_facet(facet);
    }
    item
}

fn manim_items(
    manim: &shrimply_inspector_core::manim_parameters::ManimPresentation,
) -> impl Iterator<Item = InspectorListItem> {
    let mut items = Vec::with_capacity(2);
    items.push(
        inspector_item(manim.main.clone())
            .reset(BasicInspectorAction::ResetManim(manim.main_reset.clone()))
            .boxed(),
    );
    if let Some((card, reset)) = manim.parameters.clone().zip(manim.parameters_reset.clone()) {
        items.push(
            inspector_item(card)
                .reset(BasicInspectorAction::ResetManimParameters(reset))
                .boxed(),
        );
    }
    items.into_iter()
}
