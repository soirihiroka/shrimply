use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_components_gtk::{tr, ui::I18nAlertDialogExt};
use shrimply_timeline_skia::caption_speech::{Handle, Options, Plan, Summary, Update};

use crate::{preferences::store as preferences_store, runtime::TimelineRuntime};

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);
const DIALOG_WIDTH: i32 = 720;
const DIALOG_HEIGHT: i32 = 640;

pub(super) fn show_dialog(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: preferences_store::SharedPreferences,
    plan: Plan,
) {
    let settings = Rc::new(RefCell::new(shrimply_tts::TtsSettings::default()));
    let models = Rc::new(RefCell::new(Vec::<shrimply_tts::TtsModel>::new()));
    let configuration = shrimply_tts_gtk::caption_configuration(
        preferences.clone(),
        settings.clone(),
        models.clone(),
        Rc::new(|_| {}),
        Rc::new(|| {}),
    );
    let summary = plan.description();
    let generate = adw::ButtonRow::builder()
        .title(tr!("Generate Speech").as_ref())
        .build();
    generate.add_css_class("suggested-action");
    let action_group = adw::PreferencesGroup::builder()
        .description(&summary)
        .build();
    action_group.add(&generate);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&configuration);
    content.append(&action_group);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.add(&content);
    page.add(&group);
    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Generate Speech").as_ref())
        .search_enabled(false)
        .content_width(DIALOG_WIDTH)
        .content_height(DIALOG_HEIGHT)
        .build();
    dialog.add(&page);

    let area_for_generation = area.clone();
    let runtime_for_generation = runtime.clone();
    let dialog_for_generation = dialog.clone();
    generate.connect_activated(move |_| {
        let current = settings.borrow().clone();
        let Some(model) = current.model.as_ref().and_then(|id| {
            models
                .borrow()
                .iter()
                .find(|model| &model.id == id)
                .cloned()
        }) else {
            super::interaction::show_error_dialog(
                &area_for_generation,
                "Could not generate speech",
                "The server has not provided a text-to-speech model.",
            );
            return;
        };
        let server_url = preferences_store::snapshot(&preferences).compute_server_url;
        let result = runtime_for_generation
            .borrow_mut()
            .scene
            .start_caption_speech(
                plan.clone(),
                Options {
                    server_url,
                    model: model.clone(),
                    settings: current,
                },
            );
        match result {
            Ok(handle) => {
                dialog_for_generation.close();
                show_progress(
                    &area_for_generation,
                    &runtime_for_generation,
                    handle,
                    &model.label,
                );
            }
            Err(error) => super::interaction::show_error_dialog(
                &area_for_generation,
                "Could not generate speech",
                &error,
            ),
        }
    });
    dialog.present(Some(area.upcast_ref::<gtk::Widget>()));
}

fn show_progress(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    handle: Handle,
    model: &str,
) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    let spinner = adw::Spinner::new();
    spinner.set_halign(gtk::Align::Center);
    spinner.set_size_request(32, 32);
    let chunk = gtk::Label::new(Some(tr!("Sending request…").as_ref()));
    let preview = gtk::Label::new(None);
    preview.add_css_class("dim-label");
    preview.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let model = gtk::Label::new(Some(model));
    model.add_css_class("dim-label");
    content.append(&model);
    content.append(&spinner);
    content.append(&chunk);
    content.append(&preview);
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Generating Speech…").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel")]);
    dialog.set_close_response("cancel");
    let cancellation = handle.clone();
    dialog.clone().choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |_| cancellation.cancel(),
    );
    let area = area.clone();
    let runtime = runtime.clone();
    glib::timeout_add_local(POLL_INTERVAL, move || {
        runtime.borrow_mut().scene.update_media();
        match runtime.borrow_mut().scene.take_caption_speech_update() {
            Some(Update::Progress {
                current,
                total,
                message,
                preview: text,
            }) => {
                chunk.set_label(&format!("Chunk {current}/{total} · {message}"));
                preview.set_label(&text);
                glib::ControlFlow::Continue
            }
            Some(Update::Finished(summary)) => {
                dialog.close();
                area.queue_render();
                if needs_summary(&summary) {
                    super::interaction::show_error_dialog(
                        &area,
                        "Speech generation finished",
                        &summary.to_string(),
                    );
                }
                glib::ControlFlow::Break
            }
            Some(Update::Failed(error)) => {
                dialog.close();
                super::interaction::show_error_dialog(&area, "Could not generate speech", &error);
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}

fn needs_summary(summary: &Summary) -> bool {
    summary.failed > 0 || summary.skipped > 0 || summary.unattempted > 0 || summary.cancelled
}
