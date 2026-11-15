use shrimply_inspector_core::{
    ControlKind, InspectorControl, InspectorSection,
    info::{SourceMetadata, metadata::MetadataState},
};

pub(super) fn section(
    state: &MetadataState,
    selected: SourceMetadata,
    alpha_mask: Option<u32>,
) -> InspectorSection {
    let mut section = InspectorSection::default();
    match state {
        MetadataState::Loading => section.add(
            InspectorControl::new(ControlKind::InfoLoading, "", "File Metadata")
                .value("Loading…")
                .read_only(),
        ),
        MetadataState::Failed(error) => {
            section.add(
                InspectorControl::new(ControlKind::InfoHeading, "", "Diagnostics").read_only(),
            );
            section.add(
                InspectorControl::new(ControlKind::ReadOnly, "", "Metadata")
                    .value(error)
                    .read_only(),
            );
        }
        MetadataState::Ready(info) => {
            let stream = match selected {
                SourceMetadata::Audio(index) => Some(("audio", "Audio", index)),
                SourceMetadata::Video(index) => Some(("video", "Video", index)),
                SourceMetadata::None => None,
            };
            if let Some((kind, label, selected)) = stream {
                let count = if kind == "audio" {
                    info.audio_stream_count
                } else {
                    info.video_stream_count
                };
                if count > 1 {
                    let selected = selected.min(count - 1);
                    section.add(shrimply_inspector_core::selector::selector(
                        "/track_id",
                        format!("{label} Stream"),
                        selected.to_string(),
                        (0..count).map(|index| {
                            (index.to_string(), format!("{label} stream {}", index + 1))
                        }),
                    ));
                    if kind == "video" {
                        section.add(shrimply_inspector_core::selector::optional_number_selector(
                            "/alpha_mask_video",
                            "Alpha Mask Stream",
                            alpha_mask.filter(|index| *index != selected),
                            (0..count).filter(|index| *index != selected).map(|index| {
                                (index.to_string(), format!("Video stream {}", index + 1))
                            }),
                        ));
                    }
                }
            }
            if let Some(artwork) = &info.artwork {
                let mut control =
                    InspectorControl::new(ControlKind::InfoArtwork, "", "Artwork").read_only();
                control.image_bytes = Some(artwork.clone());
                control.image_version = info.artwork_version.clone();
                section.add(control);
            }
            for group in &info.presentation.groups {
                section.add(
                    InspectorControl::new(
                        ControlKind::InfoHeading,
                        "",
                        group.display_title(selected),
                    )
                    .read_only(),
                );
                for row in &group.rows {
                    section.add(
                        InspectorControl::new(ControlKind::ReadOnly, "", &row.label)
                            .value(&row.value)
                            .read_only(),
                    );
                }
            }
        }
    }
    section
}
