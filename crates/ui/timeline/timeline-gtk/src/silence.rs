use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::gio;
use shrimply_components_gtk::{tr, ui::I18nAlertDialogExt};
use shrimply_timeline_skia::silence::Config;

use crate::TimelineRuntime;

pub(super) fn show_dialog(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let defaults = Config::default();
    let [
        threshold,
        min_silence,
        gap_tolerance,
        padding,
        delete_chunks,
    ] = shrimply_timeline_skia::silence::FIELDS.map(|field| {
        shrimply_components_gtk::ui::number_field_row(
            &field,
            defaults
                .field_value(field.key)
                .parse()
                .expect("silence default"),
        )
    });

    let group = adw::PreferencesGroup::new();
    group.add(&threshold);
    group.add(&min_silence);
    group.add(&gap_tolerance);
    group.add(&padding);
    group.add(&delete_chunks);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    content.append(&group);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Remove Silences").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("remove", "Remove Silences")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("remove"));
    dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);

    let area = area.clone();
    let runtime = runtime.clone();
    let parent = area.clone();
    dialog.choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() != "remove" {
                return;
            }
            let result = (|| {
                let mut config = Config::default();
                for (field, row) in shrimply_timeline_skia::silence::FIELDS.iter().zip([
                    &threshold,
                    &min_silence,
                    &gap_tolerance,
                    &padding,
                    &delete_chunks,
                ]) {
                    config.set_field(field.key, &row.value().to_string())?;
                }
                runtime.borrow_mut().scene.remove_silences(config)
            })();
            match result {
                Ok(0) => show_info_dialog(
                    &area,
                    "No Silences Found",
                    "No selected-audio gaps matched the configured threshold and duration.",
                ),
                Ok(_) => area.queue_render(),
                Err(error) => show_info_dialog(&area, "Could Not Remove Silences", &error),
            }
        },
    );
}

fn show_info_dialog(area: &gtk::GLArea, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog.choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        |_| {},
    );
}
