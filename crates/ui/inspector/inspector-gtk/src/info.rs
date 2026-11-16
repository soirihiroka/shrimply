use std::path::PathBuf;

use adw::prelude::*;
use gtk::glib;
use shrimply_components_gtk::tr;

pub(super) use shrimply_components_gtk::ui::property_row;

pub(super) fn file_row(path: PathBuf, label: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .title(tr!(label).as_ref())
        .subtitle(path.to_string_lossy())
        .subtitle_selectable(true)
        .subtitle_lines(0)
        .use_markup(false)
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

pub(super) fn artwork(
    data: impl AsRef<[u8]> + Send + 'static,
    label: &str,
) -> Option<adw::PreferencesGroup> {
    let texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(data)).ok()?;
    let group = adw::PreferencesGroup::builder()
        .title(tr!(label).as_ref())
        .build();
    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_height_request(shrimply_inspector_core::info::ARTWORK_PREVIEW_HEIGHT as i32);
    group.add(&picture);
    Some(group)
}

pub(super) fn format_date(seconds: i64) -> Option<String> {
    glib::DateTime::from_unix_local(seconds)
        .ok()
        .and_then(|date| date.format("%x %X").ok())
        .map(|date| date.to_string())
}
