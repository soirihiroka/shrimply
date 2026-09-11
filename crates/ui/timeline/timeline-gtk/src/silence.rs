use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::gio;
use shrimply_components_gtk::{tr, ui::I18nAlertDialogExt};
use shrimply_timeline_skia::{project::Time, silence::Config};

use crate::TimelineRuntime;

pub(super) fn show_dialog(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let defaults = Config::default();
    let threshold = adw::SpinRow::with_range(-90.0, 0.0, 1.0);
    threshold.set_title(tr!("Silence threshold").as_ref());
    threshold.set_value(defaults.threshold_db);
    threshold.set_digits(1);

    let min_silence = adw::SpinRow::with_range(0.0, 10.0, 0.05);
    min_silence.set_title(tr!("Minimum silence").as_ref());
    min_silence.set_value(defaults.min_silence.as_secs_f64());
    min_silence.set_digits(2);

    let gap_tolerance = adw::SpinRow::with_range(0.0, 5.0, 0.01);
    gap_tolerance.set_title(tr!("Gap tolerance").as_ref());
    gap_tolerance.set_value(defaults.gap_tolerance.as_secs_f64());
    gap_tolerance.set_digits(2);

    let padding = adw::SpinRow::with_range(0.0, 2.0, 0.01);
    padding.set_title(tr!("Padding").as_ref());
    padding.set_value(defaults.padding.as_secs_f64());
    padding.set_digits(2);

    let delete_chunks = adw::SpinRow::with_range(0.0, 10.0, 0.05);
    delete_chunks.set_title(tr!("Min chunk").as_ref());
    delete_chunks.set_value(defaults.delete_chunks.as_secs_f64());
    delete_chunks.set_digits(2);

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
            let result = runtime.borrow_mut().scene.remove_silences(Config {
                threshold_db: threshold.value(),
                min_silence: Time::from_seconds_f64(min_silence.value()),
                gap_tolerance: Time::from_seconds_f64(gap_tolerance.value()),
                padding: Time::from_seconds_f64(padding.value()),
                delete_chunks: Time::from_seconds_f64(delete_chunks.value()),
            });
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
