mod audio;
mod audio_generator;
mod audio_modifiers;
mod caption;
mod metadata;
mod modifiers;
mod project;
pub mod remote;
mod track;
mod video;

use serde_json::Value;
use shrimply_inspector_core::{
    ControlKind, InspectorController, InspectorDetail, InspectorSection, InspectorSnapshot,
    InspectorTarget,
};
use shrimply_project_document::project::ItemAddress;

pub use shrimply_inspector_core::BasicInspectorAction;

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorDocument {
    pub target: InspectorTarget,
    pub title: String,
    pub categories: Vec<InspectorCategory>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorCategory {
    pub key: &'static str,
    pub label: &'static str,
    pub icon: CategoryIcon,
    pub items: Vec<InspectorListItem>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CategoryIcon {
    Project,
    Track,
    Text,
    Visual,
    Audio,
    Playback,
    Info,
    Performance,
    Transition,
}

impl CategoryIcon {
    pub const fn symbolic_name(self) -> &'static str {
        match self {
            Self::Project | Self::Track => "sliders-horizontal-symbolic",
            Self::Text => "rich-text-symbolic",
            Self::Visual => "image-symbolic",
            Self::Audio => "audio-volume-high-symbolic",
            Self::Playback => "media-playback-start-symbolic",
            Self::Info => "info-outline-symbolic",
            Self::Performance => "speedometer-symbolic",
            Self::Transition => "select-symbolic",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InspectorItem {
    pub presentation: shrimply_inspector_core::item::InspectorItemPresentation,
    pub section: InspectorSection,
    pub reset: Option<shrimply_inspector_core::BasicInspectorAction>,
    pub actions: Vec<
        shrimply_inspector_core::item::HeaderAction<shrimply_inspector_core::BasicInspectorAction>,
    >,
    pub toggle: Option<
        shrimply_inspector_core::item::HeaderToggle<shrimply_inspector_core::BasicInspectorAction>,
    >,
    pub button_toggle: Option<
        shrimply_inspector_core::item::HeaderButtonToggle<
            shrimply_inspector_core::document::InspectorToggleAction,
        >,
    >,
}

impl InspectorItem {
    pub fn new(
        key: impl Into<String>,
        title: impl Into<String>,
        section: InspectorSection,
    ) -> Self {
        Self {
            presentation: shrimply_inspector_core::item::InspectorItemPresentation::new(key, title),
            section,
            reset: None,
            actions: Vec::new(),
            toggle: None,
            button_toggle: None,
        }
    }

    pub fn reset(mut self, reset: shrimply_inspector_core::BasicInspectorAction) -> Self {
        self.reset = Some(reset);
        self
    }

    pub fn toggle(
        mut self,
        toggle: shrimply_inspector_core::item::HeaderToggle<
            shrimply_inspector_core::BasicInspectorAction,
        >,
    ) -> Self {
        self.toggle = Some(toggle);
        self
    }

    pub fn alpha_mask(
        mut self,
        target: shrimply_project_document::project::VisualAlphaMaskTarget,
        mask: &shrimply_inspector_core::AlphaMaskPresentation,
    ) -> Self {
        self.section
            .controls
            .extend(mask.section.controls.iter().cloned());
        self.button_toggle = Some(shrimply_inspector_core::item::HeaderButtonToggle {
            icon: "select-symbolic",
            active: mask.active,
            tooltip: "Mask",
            activate: shrimply_inspector_core::document::InspectorToggleAction::AlphaMask(target),
        });
        self
    }

    pub fn preview_facet(mut self, facet: shrimply_preview_provider_skia::PreviewFacetKey) -> Self {
        self.presentation = self.presentation.preview_facet(facet);
        self
    }

    pub fn boxed(self) -> InspectorListItem {
        InspectorListItem::Item(Box::new(self))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum InspectorListItem {
    Item(Box<InspectorItem>),
    Flat(InspectorSection),
}

pub fn basic(snapshot: InspectorSnapshot) -> InspectorDocument {
    let categories = match &snapshot.target {
        InspectorTarget::Project => project::categories(
            snapshot
                .project
                .as_ref()
                .expect("project snapshot must include project presentation"),
        ),
        InspectorTarget::Track(_) => track::categories(
            snapshot
                .track
                .as_ref()
                .expect("track snapshot must include track presentation"),
        ),
        InspectorTarget::Transition { .. } => {
            let transition = snapshot
                .transition
                .as_ref()
                .expect("transition snapshot must include transition presentation");
            vec![InspectorCategory {
                key: "transition",
                label: transition.title,
                icon: CategoryIcon::Transition,
                items: vec![
                    InspectorItem::new("transition", transition.title, transition.section())
                        .boxed(),
                ],
            }]
        }
        InspectorTarget::Item(ItemAddress::Comment { .. }) => vec![InspectorCategory {
            key: "comment",
            label: "Comment",
            icon: CategoryIcon::Text,
            items: vec![
                InspectorItem::new(
                    "comment",
                    "Comment",
                    shrimply_inspector_core::comment::section(
                        &serde_json::from_value(snapshot.value.clone())
                            .expect("comment must be valid"),
                    ),
                )
                .boxed(),
            ],
        }],
        InspectorTarget::Item(ItemAddress::Caption { .. }) => {
            caption::categories(&snapshot.value, &snapshot.details)
        }
        InspectorTarget::Item(ItemAddress::Audio { .. }) => {
            audio::categories(&snapshot.value, &snapshot.details, snapshot.runtime)
        }
        InspectorTarget::Item(ItemAddress::Video { .. }) => video::categories(
            snapshot
                .video
                .as_ref()
                .expect("video snapshot must include video presentation"),
            &snapshot.details,
        ),
    };
    InspectorDocument {
        target: snapshot.target,
        title: snapshot.title,
        categories,
    }
}

pub fn poll(controller: &InspectorController) -> bool {
    controller.poll_document_dependencies()
}

pub fn document(
    controller: &InspectorController,
    format_date: fn(i64) -> Option<String>,
    server_url: &str,
    remembered_tts_model: &str,
) -> InspectorDocument {
    let snapshot = controller.document_snapshot(server_url);
    let media = controller.document_metadata(snapshot.media.as_ref(), format_date);
    let source = snapshot.media.as_ref().map_or(
        shrimply_inspector_core::info::SourceMetadata::None,
        |media| media.selected,
    );
    let alpha_mask = snapshot
        .value
        .get("alpha_mask_video")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let mut document = basic(snapshot);
    if let Some(media) = media
        && let Some(category) = document
            .categories
            .iter_mut()
            .find(|category| category.key == "info")
    {
        category
            .items
            .push(InspectorListItem::Flat(metadata::section(
                &media, source, alpha_mask,
            )));
    }
    for category in &mut document.categories {
        for item in &mut category.items {
            let section = match item {
                InspectorListItem::Flat(section) => section,
                InspectorListItem::Item(item) => &mut item.section,
            };
            for control in &mut section.controls {
                if control.kind == ControlKind::TtsEditor {
                    match controller.tts_editor(&document.target, server_url, remembered_tts_model)
                    {
                        Ok(presentation) => control.tts = Some(Box::new(presentation)),
                        Err(error) => {
                            control.subtitle = error;
                            control.sensitive = false;
                        }
                    }
                }
                if control.kind == ControlKind::VoiceModel {
                    controller.populate_voice_model_control(server_url, control);
                }
                let clipboard = controller.property_clipboard();
                let can_paste = match control.kind {
                    ControlKind::AudioModifierMenu => {
                        controller.can_paste_audio_modifiers(&document.target, &clipboard)
                    }
                    ControlKind::VisualModifierMenu => {
                        controller.can_paste_visual_modifiers(&document.target, &clipboard)
                    }
                    _ => continue,
                };
                if can_paste {
                    control.values.push("__paste__".into());
                    control.labels.push("Paste modifiers".into());
                    control.search_terms.push("Paste copied modifiers".into());
                }
            }
        }
    }
    document
}

fn detail_item(details: &[InspectorDetail]) -> InspectorListItem {
    let mut section = InspectorSection::default();
    for detail in details {
        section.add(
            shrimply_inspector_core::InspectorControl::new(
                if matches!(detail.label, "File Location" | "Project File") {
                    ControlKind::FileLocation
                } else {
                    ControlKind::ReadOnly
                },
                "",
                detail.label,
            )
            .value(&detail.value)
            .read_only(),
        );
    }
    InspectorListItem::Flat(section)
}

fn text<'a>(value: &'a Value, path: &str) -> &'a str {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("inspector text is unavailable: {path}"))
}
