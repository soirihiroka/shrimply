use super::*;
use shrimply_components_gtk::{tr, ui::I18nAlertDialogExt};
pub(super) use shrimply_timeline_skia::audio_selection::selected_audio_project;
use shrimply_timeline_skia::transcription::{Handle, Options, Update};

pub(super) fn add_caption_item_context_actions(
    actions: &gio::SimpleActionGroup,
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
) {
    add_menu_action(actions, "generate-speech", {
        let area = area.clone();
        let runtime = runtime.clone();
        let preferences = preferences.clone();
        move || {
            let url = preferences_store::snapshot(&preferences).compute_server_url;
            if url.is_empty() {
                show_error_dialog(
                    &area,
                    "Compute server is not configured",
                    "Set the Server URL in Preferences before generating speech.",
                );
                return;
            }
            let plan = match runtime.borrow().scene.caption_speech_plan() {
                Ok(plan) => plan,
                Err(error) => {
                    show_error_dialog(&area, "Could not generate speech", &error);
                    return;
                }
            };
            caption_tts::show_dialog(&area, &runtime, preferences.clone(), plan);
        }
    });
}

pub(super) fn show_transcribe_dialog(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: &preferences_store::SharedPreferences,
) {
    let server_url = preferences_store::snapshot(preferences).compute_server_url;
    if server_url.trim().is_empty() {
        show_error_dialog(
            area,
            "Compute server is not configured",
            "Set the Server URL in Preferences before transcribing audio.",
        );
        return;
    }
    let (sender, receiver) = async_channel::bounded(1);
    let probe_url = server_url.clone();
    thread::spawn(move || {
        let _ = sender.send_blocking(shrimply_timeline_skia::transcription::models(&probe_url));
    });
    let area = area.clone();
    let runtime = runtime.clone();
    let preferences = preferences.clone();
    glib::spawn_future_local(async move {
        match receiver.recv().await {
            Ok(Ok(models)) => show_options(&area, &runtime, preferences, server_url, models),
            Ok(Err(error)) => show_error_dialog(&area, "Could not connect to server", &error),
            Err(_) => show_error_dialog(
                &area,
                "Could not connect to server",
                "Server status check stopped unexpectedly.",
            ),
        }
    });
}

fn show_options(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    preferences: preferences_store::SharedPreferences,
    server_url: String,
    models: Vec<String>,
) {
    let model_labels = models.iter().map(String::as_str).collect::<Vec<_>>();
    let model_list = gtk::StringList::new(&model_labels);
    let model = adw::ComboRow::new();
    model.set_title(tr!("Model").as_ref());
    model.set_model(Some(&model_list));
    let last_model = preferences_store::snapshot(&preferences).last_stt_model;
    model.set_selected(
        models
            .iter()
            .position(|candidate| candidate == &last_model)
            .unwrap_or(0) as u32,
    );

    shrimply_components_gtk::ui::adaptive_choice(&model);

    let defaults = Options::default();
    let chunked = adw::SwitchRow::new();
    chunked.set_title(tr!(shrimply_timeline_skia::transcription::FOLLOW_CUTS_LABEL).as_ref());
    chunked.set_active(defaults.chunked);
    let snap_labels = shrimply_timeline_skia::transcription::SNAP_SOURCES
        .iter()
        .map(|(_, label, _)| tr!(*label))
        .collect::<Vec<_>>();
    let snap_labels = snap_labels
        .iter()
        .map(|label| label.as_ref())
        .collect::<Vec<_>>();
    let snap_list = gtk::StringList::new(&snap_labels);
    let snap = adw::ComboRow::new();
    snap.set_title(tr!(shrimply_timeline_skia::transcription::SNAP_SOURCE_LABEL).as_ref());
    snap.set_model(Some(&snap_list));

    shrimply_components_gtk::ui::adaptive_choice(&snap);

    let [tolerance, threshold] =
        shrimply_timeline_skia::transcription::NUMBER_FIELDS.map(|field| {
            shrimply_components_gtk::ui::number_field_row(&field, defaults.field_value(field.key))
        });

    let count = gtk::Label::new(None);
    count.set_halign(gtk::Align::Start);
    count.add_css_class("dim-label");
    let update_count = {
        let runtime = runtime.clone();
        let count = count.clone();
        let chunked = chunked.clone();
        let snap = snap.clone();
        let tolerance = tolerance.clone();
        let threshold = threshold.clone();
        move || {
            let options = options(&chunked, &snap, &tolerance, &threshold);
            let label = match runtime.borrow().scene.transcription_chunk_count(options) {
                Ok(1) => "1 chunk will be transcribed.".into(),
                Ok(chunks) => format!("{chunks} chunks will be transcribed."),
                Err(error) => error,
            };
            count.set_label(&label);
        }
    };
    update_count();
    let update = update_count.clone();
    chunked.connect_active_notify(move |_| update());
    threshold.connect_value_notify(move |_| update_count());

    let group = adw::PreferencesGroup::new();
    group.add(&model);
    group.add(&chunked);
    group.add(&snap);
    group.add(&tolerance);
    group.add(&threshold);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_margin_top(6);
    content.set_margin_bottom(6);
    content.set_margin_start(6);
    content.set_margin_end(6);
    content.append(&group);
    count.set_margin_top(8);
    content.append(&count);

    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Transcribe").as_ref())
        .extra_child(&content)
        .build();
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("transcribe", "Transcribe")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("transcribe"));
    dialog.set_response_appearance("transcribe", adw::ResponseAppearance::Suggested);
    let area = area.clone();
    let runtime = runtime.clone();
    dialog.choose(
        Some(area.clone().upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() != "transcribe" {
                return;
            }
            let model = models
                .get(model.selected() as usize)
                .cloned()
                .expect("speech-to-text model selection must be valid");
            preferences_store::set_last_stt_model(&preferences, &model);
            match runtime.borrow_mut().scene.start_transcription(
                options(&chunked, &snap, &tolerance, &threshold),
                server_url.clone(),
                model.clone(),
            ) {
                Ok(handle) => show_progress(&area, &runtime, handle, &model),
                Err(error) => show_error_dialog(&area, "Could not transcribe", &error),
            }
        },
    );
}

fn options(
    chunked: &adw::SwitchRow,
    snap: &adw::ComboRow,
    tolerance: &adw::SpinRow,
    threshold: &adw::SpinRow,
) -> Options {
    let mut options = Options::default();
    for (key, value) in [
        ("chunked", chunked.is_active().to_string()),
        (
            "snap_source",
            shrimply_timeline_skia::transcription::SNAP_SOURCES[snap.selected() as usize]
                .0
                .to_owned(),
        ),
        ("snap_tolerance", tolerance.value().to_string()),
        ("continue_threshold", threshold.value().to_string()),
    ] {
        options
            .set_field(key, &value)
            .expect("native transcription values are bounded");
    }
    options
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
    let status = gtk::Label::new(Some(tr!("Sending request…").as_ref()));
    status.set_halign(gtk::Align::Center);
    let model = gtk::Label::new(Some(model));
    model.add_css_class("dim-label");
    content.append(&model);
    content.append(&spinner);
    content.append(&status);
    let dialog = adw::AlertDialog::builder()
        .heading(tr!("Transcribing...").as_ref())
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
    glib::timeout_add_local(WAVEFORM_POLL_INTERVAL, move || {
        runtime.borrow_mut().scene.update_media();
        let update = runtime.borrow_mut().scene.take_transcription_update();
        match update {
            Some(Update::Progress(message)) => {
                status.set_label(&message);
                glib::ControlFlow::Continue
            }
            Some(Update::Finished { .. }) => {
                dialog.close();
                area.queue_render();
                glib::ControlFlow::Break
            }
            Some(Update::Cancelled) => {
                dialog.close();
                glib::ControlFlow::Break
            }
            Some(Update::Failed(error)) => {
                dialog.close();
                show_error_dialog(&area, "Could not transcribe", &error);
                glib::ControlFlow::Break
            }
            None => glib::ControlFlow::Continue,
        }
    });
}
