use std::{path::PathBuf, thread};

use adw::prelude::*;
use ffmpeg::{format, media};
use ffmpeg_next as ffmpeg;
use glam::UVec2;
use gtk::glib;
use shrimply_components_gtk::tr;
use shrimply_media_info::FileInfo;
use shrimply_project_document::project::{Asset, Time};

use super::{
    InspectorContext,
    item::{InspectorListItem, flat},
};

pub(super) use shrimply_inspector_core::info::SourceMetadata;

const ARTWORK_HEIGHT: i32 = 220;

pub(super) struct ItemInfo {
    pub(super) leading: Vec<gtk::Widget>,
    pub(super) kind: &'static str,
    pub(super) natural_duration: Option<Time>,
    pub(super) start: Time,
    pub(super) end: Time,
    pub(super) source_offset: Option<Time>,
    pub(super) dimensions: Option<UVec2>,
    pub(super) file: Option<Asset>,
    pub(super) source_metadata: SourceMetadata,
}

pub(super) fn item(context: &InspectorContext, info: ItemInfo) -> InspectorListItem {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let summary = adw::PreferencesGroup::new();
    for leading in info.leading {
        summary.add(&leading);
    }
    add_row(&summary, "Type", info.kind);
    if let Some(address) = &context.selected_item {
        add_row(&summary, "Item ID", &address.item_id().to_string());
        add_row(&summary, "Track ID", &address.track_id().to_string());
        if !address.sequence_path().is_empty() {
            add_row(
                &summary,
                "Sequence Path",
                &address
                    .sequence_path()
                    .iter()
                    .map(uuid::Uuid::to_string)
                    .collect::<Vec<_>>()
                    .join(" / "),
            );
        }
        if let Some((start, end)) = context.project.borrow().projected_item_times(address) {
            add_row(
                &summary,
                "Timeline Start",
                &crate::time_format::playback_time(start),
            );
            add_row(
                &summary,
                "Timeline End",
                &crate::time_format::playback_time(end),
            );
        }
    }
    add_row(
        &summary,
        "Local Start",
        &crate::time_format::playback_time(info.start),
    );
    add_row(
        &summary,
        "Local End",
        &crate::time_format::playback_time(info.end),
    );
    if let Some(duration) = info.natural_duration {
        add_row(
            &summary,
            "Natural Duration",
            &crate::time_format::playback_time(duration),
        );
    }
    add_row(
        &summary,
        "Timeline Duration",
        &crate::time_format::playback_time(info.end.saturating_sub(info.start)),
    );
    if let Some(offset) = info.source_offset {
        add_row(
            &summary,
            "Source Offset",
            &crate::time_format::playback_time(offset),
        );
    }
    if let Some(size) = info.dimensions.filter(|size| size.x > 0 && size.y > 0) {
        add_row(&summary, "Dimensions", &format!("{} × {}", size.x, size.y));
    }
    content.append(&summary);

    if let Some(file) = info.file.filter(|file| !file.path().as_os_str().is_empty()) {
        content.append(&file_row(file.path().to_path_buf()));
        let metadata = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let loading = adw::PreferencesGroup::new();
        let loading_row = adw::ActionRow::builder()
            .title(tr!("File Metadata").as_ref())
            .subtitle(tr!("Loading…").as_ref())
            .build();
        loading_row.add_suffix(&adw::Spinner::new());
        loading.add(&loading_row);
        metadata.append(&loading);
        content.append(&metadata);

        let revision = file.snapshot().ok().map(|snapshot| snapshot.revision());
        let path = file.path().to_path_buf();
        let (sender, receiver) = async_channel::bounded(1);
        thread::spawn(move || {
            let _ = sender.send_blocking(shrimply_media_info::inspect(&path, revision));
        });
        let weak_metadata = metadata.downgrade();
        glib::spawn_future_local(async move {
            let Ok(result) = receiver.recv().await else {
                return;
            };
            let Some(metadata) = weak_metadata.upgrade() else {
                return;
            };
            while let Some(child) = metadata.first_child() {
                metadata.remove(&child);
            }
            match result {
                Ok(file) => render_file_info(&metadata, &file, info.source_metadata),
                Err(error) => {
                    let group = adw::PreferencesGroup::builder()
                        .title(tr!("Diagnostics").as_ref())
                        .build();
                    add_row(&group, "Metadata", &error);
                    metadata.append(&group);
                }
            }
        });
    }

    flat(content)
}

fn file_row(path: PathBuf) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .title(tr!("File Location").as_ref())
        .subtitle(path.to_string_lossy())
        .build();
    let button = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(tr!("Show in folder").as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.connect_clicked(move |button| {
        if let Err(error) =
            crate::desktop_open::show_path_in_folder(button.upcast_ref(), path.clone())
        {
            let dialog = adw::AlertDialog::new(Some("Could not show media file"), Some(&error));
            dialog.add_response("close", tr!("Close").as_ref());
            dialog.present(Some(button));
        }
    });
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    group.add(&row);
    group
}

fn render_file_info(container: &gtk::Box, info: &FileInfo, selected: SourceMetadata) {
    let presentation = shrimply_inspector_core::info::media_info_presentation(info, format_date);
    if let Some(artwork) = presentation.artwork
        && let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(artwork.data))
    {
        let group = adw::PreferencesGroup::builder()
            .title(tr!("Artwork").as_ref())
            .build();
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_content_fit(gtk::ContentFit::Contain);
        picture.set_can_shrink(true);
        picture.set_height_request(ARTWORK_HEIGHT);
        group.add(&picture);
        container.append(&group);
    }
    for info_group in presentation.groups {
        let group = adw::PreferencesGroup::builder()
            .title(tr!(&info_group.display_title(selected)).as_ref())
            .build();
        for row in info_group.rows {
            add_row(&group, &row.label, &row.value);
        }
        container.append(&group);
    }
}

fn add_row(group: &adw::PreferencesGroup, title: &str, value: &str) {
    group.add(
        &adw::ActionRow::builder()
            .title(tr!(title).as_ref())
            .subtitle(value)
            .build(),
    );
}

fn format_date(seconds: i64) -> Option<String> {
    glib::DateTime::from_unix_local(seconds)
        .ok()
        .and_then(|date| date.format("%x %X").ok())
        .map(|date| date.to_string())
}

pub(super) fn video_stream_count(file: &std::path::Path) -> usize {
    format::input(file).map_or(0, |input| {
        input
            .streams()
            .filter(|stream| stream.parameters().medium() == media::Type::Video)
            .filter(|stream| {
                !stream
                    .disposition()
                    .contains(format::stream::Disposition::ATTACHED_PIC)
            })
            .count()
    })
}

pub(super) fn audio_stream_count(file: &std::path::Path) -> usize {
    format::input(file).map_or(0, |input| {
        input
            .streams()
            .filter(|stream| stream.parameters().medium() == media::Type::Audio)
            .count()
    })
}
