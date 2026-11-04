use adw::prelude::*;
use shrimply_components_gtk::{desktop_open, tr};
use shrimply_trust_core::{Entry, Kind, Review};

pub fn page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(tr!("Trust").as_ref())
        .icon_name("security-high-symbolic")
        .build();
    let group = adw::PreferencesGroup::builder()
        .title(tr!("Trusted sources").as_ref())
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    rebuild_rows(&list);
    group.add(&list);
    page.add(&group);
    page
}

fn rebuild_rows(list: &gtk::ListBox) {
    list.remove_all();
    match shrimply_trust_core::entries() {
        Err(error) => list.append(
            &adw::ActionRow::builder()
                .title(tr!("Could not read trusted locations").as_ref())
                .subtitle(&error)
                .use_markup(false)
                .build(),
        ),
        Ok(entries) => {
            for entry in entries {
                let row = adw::ActionRow::builder()
                    .title(entry.path.to_string_lossy())
                    .use_markup(false)
                    .build();
                let (icon, label) = match entry.kind {
                    Kind::File => ("text-x-generic-symbolic", tr!("File")),
                    Kind::Folder => ("folder-symbolic", tr!("Folder")),
                };
                let icon = gtk::Image::from_icon_name(icon);
                icon.set_tooltip_text(Some(label.as_ref()));
                row.add_suffix(&icon);
                row.add_suffix(&source_menu(&row, list, entry));
                list.append(&row);
            }
        }
    }
    let add = adw::ButtonRow::builder()
        .title(tr!("Add source").as_ref())
        .start_icon_name("list-add-symbolic")
        .build();
    let weak_list = list.downgrade();
    add.connect_activated(move |button| {
        let Some(list) = weak_list.upgrade() else {
            return;
        };
        choose_source(button, &list);
    });
    list.append(&add);
}

fn source_menu(row: &adw::ActionRow, list: &gtk::ListBox, entry: Entry) -> gtk::MenuButton {
    let popover = gtk::Popover::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let info = gtk::Button::with_label(tr!("Info").as_ref());
    info.add_css_class("flat");
    let info_popover = popover.downgrade();
    let info_row = row.downgrade();
    let info_entry = entry.clone();
    info.connect_clicked(move |_| {
        if let Some(popover) = info_popover.upgrade() {
            popover.popdown();
        }
        let Some(row) = info_row.upgrade() else {
            return;
        };
        let scope = match info_entry.kind {
            Kind::File => tr!("File trust includes future edits and replacements."),
            Kind::Folder => {
                tr!("Folder trust includes all current and future files and subfolders.")
            }
        };
        let dialog = adw::AlertDialog::builder()
            .heading(tr!("Trusted source"))
            .body(format!(
                "{}\n\n{scope}\n\n{}",
                info_entry.path.display(),
                tr!("Removing trust stops affected workers.")
            ))
            .body_use_markup(false)
            .build();
        dialog.add_response("close", tr!("Close").as_ref());
        dialog.present(Some(&row));
    });
    content.append(&info);

    let show = gtk::Button::with_label(tr!("Show in File Manager").as_ref());
    show.add_css_class("flat");
    let show_popover = popover.downgrade();
    let show_row = row.downgrade();
    let path = entry.path.clone();
    show.connect_clicked(move |_| {
        if let Some(popover) = show_popover.upgrade() {
            popover.popdown();
        }
        let Some(row) = show_row.upgrade() else {
            return;
        };
        if let Err(error) = desktop_open::show_path_in_folder(row.upcast_ref(), path.clone()) {
            let dialog =
                adw::AlertDialog::new(Some(tr!("Could not show source").as_ref()), Some(&error));
            dialog.add_response("close", tr!("Close").as_ref());
            dialog.present(Some(&row));
        }
    });
    content.append(&show);

    let remove = gtk::Button::with_label(tr!("Remove").as_ref());
    remove.add_css_class("flat");
    remove.add_css_class("destructive-action");
    let remove_popover = popover.downgrade();
    let remove_row = row.downgrade();
    let list = list.downgrade();
    remove.connect_clicked(move |_| {
        if let Some(popover) = remove_popover.upgrade() {
            popover.popdown();
        }
        let (Some(list), Some(row)) = (list.upgrade(), remove_row.upgrade()) else {
            return;
        };
        match shrimply_trust_core::remove(&entry) {
            Ok(()) => list.remove(&row),
            Err(error) => {
                let dialog = adw::AlertDialog::new(
                    Some(tr!("Could not remove trust").as_ref()),
                    Some(&error),
                );
                dialog.add_response("close", tr!("Close").as_ref());
                dialog.present(Some(&row));
            }
        }
    });
    content.append(&remove);
    popover.set_child(Some(&content));

    gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .has_frame(false)
        .css_classes(["flat"])
        .tooltip_text(tr!("Source menu"))
        .valign(gtk::Align::Center)
        .popover(&popover)
        .build()
}

fn choose_source(parent: &adw::ButtonRow, list: &gtk::ListBox) {
    let dialog = adw::AlertDialog::new(
        Some(tr!("Add trusted source").as_ref()),
        Some(shrimply_trust_core::EXPLANATION),
    );
    dialog.add_response("cancel", tr!("Cancel").as_ref());
    dialog.add_response("file", tr!("Choose File").as_ref());
    dialog.add_response("folder", tr!("Choose Folder").as_ref());
    dialog.set_close_response("cancel");
    let list = list.downgrade();
    dialog.choose(
        Some(parent),
        None::<&gtk::gio::Cancellable>,
        move |answer| {
            let kind = match answer.as_str() {
                "file" => Kind::File,
                "folder" => Kind::Folder,
                _ => return,
            };
            let Some(list) = list.upgrade() else {
                return;
            };
            let picker = gtk::FileDialog::builder()
                .title(tr!("Add trusted source"))
                .accept_label(tr!("Trust"))
                .modal(true)
                .build();
            let parent = list.root().and_downcast::<gtk::Window>();
            let selected = move |result: Result<gtk::gio::File, gtk::glib::Error>| {
                let result = match result {
                    Ok(file) => file
                        .path()
                        .ok_or_else(|| tr!("Choose a local source").to_string()),
                    Err(error) if error.matches(gtk::DialogError::Dismissed) => return,
                    Err(error) => Err(error.to_string()),
                }
                .and_then(|path| path.canonicalize().map_err(|error| error.to_string()))
                .and_then(|path| {
                    let review = match kind {
                        Kind::File => Review {
                            files: vec![path],
                            folders: Vec::new(),
                        },
                        Kind::Folder => Review {
                            files: Vec::new(),
                            folders: vec![path],
                        },
                    };
                    review.approve(kind)
                });
                match result {
                    Ok(()) => rebuild_rows(&list),
                    Err(error) => {
                        let dialog = adw::AlertDialog::new(
                            Some(tr!("Could not add trusted source").as_ref()),
                            Some(&error),
                        );
                        dialog.add_response("close", tr!("Close").as_ref());
                        dialog.present(Some(&list));
                    }
                }
            };
            match kind {
                Kind::File => shrimply_components_gtk::file_picker::open(
                    "Add trusted source",
                    &picker,
                    parent.as_ref(),
                    selected,
                ),
                Kind::Folder => {
                    picker.select_folder(parent.as_ref(), None::<&gtk::gio::Cancellable>, selected)
                }
            }
        },
    );
}
