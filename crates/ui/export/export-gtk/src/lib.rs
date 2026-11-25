use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nMenuExt;
pub use shrimply_export_core::audio;
use shrimply_export_core::job::{Event, Job, Outcome};
use shrimply_export_core::output;
pub use shrimply_export_cuda as video;

pub use shrimply_components_gtk::desktop_open;
pub use shrimply_export_core::{caption, project, time_format};

mod captions;
use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_editor_state::preferences;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

pub fn build_export_button(
    window: &adw::ApplicationWindow,
    project: Rc<RefCell<project::Project>>,
    preferences: preferences::SharedPreferences,
) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append_i18n("Export video", "export.video");
    menu.append_i18n("Export Captions…", "export.captions");

    let actions = gio::SimpleActionGroup::new();
    add_menu_action(&actions, "video", {
        let window = window.clone();
        let project = project.clone();
        let preferences = preferences.clone();
        move || open_export_page(&window, project.clone(), preferences.clone())
    });
    add_menu_action(&actions, "captions", {
        let window = window.clone();
        let project = project.clone();
        move || captions::open_dialog(&window, project.clone())
    });

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.insert_action_group("export", Some(&actions));

    let button = gtk::MenuButton::builder()
        .icon_name("share-symbolic")
        .tooltip_text(tr!("Export").as_ref())
        .has_frame(false)
        .popover(&popover)
        .build();
    shrimply_components_gtk::ui::adaptive_menu(&button, "export", &actions);
    button.add_css_class("flat");
    button
}

fn add_menu_action<F>(group: &gio::SimpleActionGroup, name: &str, activate: F)
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    group.add_action(&action);
}

fn open_export_page(
    parent: &adw::ApplicationWindow,
    project: Rc<RefCell<project::Project>>,
    preferences: preferences::SharedPreferences,
) {
    use shrimply_export_core::video::settings;
    let backend = video::settings_backend();
    let snapshot = project.borrow().clone();
    let schema = settings::schema_for(&snapshot, backend, None);
    let values = Rc::new(RefCell::new(schema["values"].clone()));
    let rows = Rc::new(RefCell::new(
        Vec::<(String, glib::WeakRef<gtk::Widget>)>::new(),
    ));
    let groups = Rc::new(RefCell::new(Vec::<(
        String,
        glib::WeakRef<adw::PreferencesGroup>,
    )>::new()));
    let refresh: Rc<dyn Fn()> = Rc::new({
        let values = values.clone();
        let rows = rows.clone();
        let groups = groups.clone();
        move || {
            let schema = settings::schema_for(&snapshot, backend, Some(&values.borrow()));
            for field in schema["fields"].as_array().expect("export fields") {
                if let Some(row) = rows
                    .borrow()
                    .iter()
                    .find(|(id, _)| field["id"] == id.as_str())
                    .and_then(|(_, row)| row.upgrade())
                {
                    row.set_visible(field["visible"].as_bool().expect("field visibility"));
                    row.set_sensitive(field["enabled"].as_bool().expect("field sensitivity"));
                }
            }
            for group in schema["groups"].as_array().expect("export groups") {
                if let Some(row) = groups
                    .borrow()
                    .iter()
                    .find(|(label, _)| group["label"] == label.as_str())
                    .and_then(|(_, row)| row.upgrade())
                {
                    row.set_visible(group["visible"].as_bool().expect("group visibility"));
                }
            }
        }
    });
    let page = adw::PreferencesPage::builder()
        .title(schema["title"].as_str().expect("export title"))
        .name("export")
        .build();
    let mut group_rows = std::collections::BTreeMap::new();
    for group in schema["groups"].as_array().expect("export groups") {
        let label = group["label"].as_str().expect("group label");
        let row = adw::PreferencesGroup::builder().title(label).build();
        groups
            .borrow_mut()
            .push((label.to_owned(), row.downgrade()));
        page.add(&row);
        group_rows.insert(label.to_owned(), row);
    }
    for field in schema["fields"].as_array().expect("export fields") {
        let id = field["id"].as_str().expect("export field id").to_owned();
        let label = field["label"].as_str().expect("export field label");
        let subtitle = field["subtitle"].as_str().unwrap_or_default();
        let update: Rc<dyn Fn(serde_json::Value)> = Rc::new({
            let values = values.clone();
            let refresh = refresh.clone();
            let id = id.clone();
            move |value| {
                values.borrow_mut()[&id] = value;
                refresh();
            }
        });
        let row: gtk::Widget = match field["widget"].as_str().expect("export widget") {
            "choice" => {
                let choices = field["choices"].as_array().expect("export choices").clone();
                let labels: Vec<_> = choices
                    .iter()
                    .map(|choice| choice["label"].as_str().expect("choice label"))
                    .collect();
                let row = adw::ComboRow::builder()
                    .title(label)
                    .subtitle(subtitle)
                    .model(&gtk::StringList::new(&labels))
                    .build();
                let selected = field["value"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| field["value"].to_string());
                row.set_selected(
                    choices
                        .iter()
                        .position(|choice| choice["value"] == selected)
                        .expect("default export choice") as u32,
                );
                shrimply_components_gtk::ui::adaptive_choice(&row);
                row.connect_selected_notify(move |row| {
                    update(
                        choices
                            .get(row.selected() as usize)
                            .expect("selected export choice")["value"]
                            .clone(),
                    );
                });
                row.upcast()
            }
            "number" => {
                let range = field["range"].as_array().expect("export range");
                let row = adw::SpinRow::with_range(
                    range[0].as_f64().expect("minimum"),
                    range[1].as_f64().expect("maximum"),
                    range[2].as_f64().expect("step"),
                );
                row.set_title(label);
                row.set_subtitle(subtitle);
                row.set_digits(0);
                row.set_value(field["value"].as_f64().expect("export default number"));
                row.connect_value_notify(move |row| update(serde_json::json!(row.value() as u64)));
                row.upcast()
            }
            "toggle" => {
                let row = adw::SwitchRow::builder()
                    .title(label)
                    .subtitle(subtitle)
                    .active(field["value"].as_bool().expect("export toggle"))
                    .build();
                row.connect_active_notify(move |row| update(serde_json::json!(row.is_active())));
                row.upcast()
            }
            _ => unreachable!("export native widget"),
        };
        rows.borrow_mut().push((id, row.downgrade()));
        group_rows
            .get(field["group"].as_str().expect("field group"))
            .expect("export group")
            .add(&row);
    }
    refresh();
    let actions = adw::PreferencesGroup::new();
    let export = adw::ButtonRow::builder()
        .title(schema["submit_label"].as_str().expect("export label"))
        .build();
    export.add_css_class("suggested-action");
    actions.add(&export);
    page.add(&actions);
    let dialog = adw::PreferencesDialog::builder()
        .title(schema["title"].as_str().expect("export title"))
        .search_enabled(false)
        .build();
    dialog.add(&page);
    export.connect_activated({
        let parent = parent.clone();
        let dialog = dialog.downgrade();
        move |_| {
            let project_snapshot = project.borrow().clone();
            let preferences = preferences::snapshot(&preferences);
            let settings = match video::parse_settings(
                &project_snapshot,
                std::path::PathBuf::new(),
                &values.borrow(),
                video::RenderResourceConfig {
                    maximum_temporal_decoders: preferences.temporal_decoder_pool_size as usize,
                    gpu_host_memory_gib: preferences.gpu_host_memory_gib,
                },
            ) {
                Ok(settings) => settings,
                Err(error) => {
                    show_export_error(&parent, "Could not export video", &error);
                    return;
                }
            };
            let label = "Export Video";
            let picker = gtk::FileDialog::builder()
                .title(tr!(label).as_ref())
                .initial_name(output::default_filename(
                    &project_snapshot,
                    video::extension_for_container(settings.container),
                ))
                .build();
            let parent_for_result = parent.clone();
            shrimply_components_gtk::file_picker::save(
                label,
                &picker,
                Some(parent.upcast_ref::<gtk::Window>()),
                move |result| {
                    let Some(file) = result.ok() else {
                        return;
                    };
                    let Some(path) = file.path() else {
                        show_export_error(
                            &parent_for_result,
                            "Could not export video",
                            "Could not resolve the selected file path.",
                        );
                        return;
                    };
                    let mut settings = settings.clone();
                    settings.path = output::ensure_extension(
                        path,
                        video::extension_for_container(settings.container),
                    );
                    start_video_export(&parent_for_result, project_snapshot.clone(), settings);
                },
            );
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
    });
    dialog.present(Some(parent.upcast_ref::<gtk::Widget>()));
}

fn start_video_export(
    parent: &adw::ApplicationWindow,
    project: project::Project,
    settings: video::ExportSettings,
) {
    let path = settings.path.clone();
    let job = Job::spawn(
        move |cancelled, progress| video::export_project(project, settings, cancelled, progress),
        || {},
    );
    let started = Instant::now();
    let dialog_parent = parent.clone();
    present_export_job(
        parent,
        "Exporting Video",
        "Preparing",
        job,
        move |outcome| match outcome {
            Outcome::Complete(()) => {
                let title = shrimply_components_gtk::i18n::text_args(
                    "Video exported in %{duration}",
                    &[("duration", time_format::human_duration(started.elapsed()))],
                );
                shrimply_components_gtk::export_feedback::show_export_finished(
                    &dialog_parent,
                    &title,
                    &path,
                );
            }
            Outcome::Failed(error) => {
                show_export_error(&dialog_parent, "Could not export video", &error)
            }
            Outcome::Cancelled => {}
        },
    );
}

pub fn present_export_job(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    label: &str,
    job: Result<Job, String>,
    on_outcome: impl Fn(Outcome<()>) + 'static,
) {
    let job = match job {
        Ok(job) => Rc::new(RefCell::new(job)),
        Err(error) => {
            on_outcome(Outcome::Failed(error));
            return;
        }
    };
    let (dialog, state, bar) = export_progress_dialog(parent, title, label);
    dialog.connect_closed({
        let job = job.clone();
        move |_| job.borrow().cancel()
    });
    glib::timeout_add_local(Duration::from_millis(100), move || {
        let outcome = job.borrow_mut().poll().find_map(|event| match event {
            Event::Progress(progress) => {
                update_export_progress(&state, &bar, progress);
                None
            }
            Event::Finished(outcome) => Some(outcome),
            Event::Started | Event::Cancelling => None,
        });
        if let Some(outcome) = outcome {
            dialog.close();
            on_outcome(outcome);
            return glib::ControlFlow::Break;
        }
        if bar.fraction() <= 0.0 {
            bar.pulse();
        }
        glib::ControlFlow::Continue
    });
}

fn export_progress_dialog(
    parent: &impl IsA<gtk::Widget>,
    title: &str,
    label: &str,
) -> (adw::Dialog, gtk::Label, gtk::ProgressBar) {
    let dialog = adw::Dialog::builder()
        .title(tr!(title).as_ref())
        .content_width(460)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let state = gtk::Label::new(Some(tr!(label).as_ref()));
    state.set_halign(gtk::Align::Center);
    state.set_wrap(true);

    let progress_box = gtk::Box::new(gtk::Orientation::Vertical, 8);

    let progress = gtk::ProgressBar::new();
    progress.set_show_text(true);
    progress.set_text(Some(tr!(label).as_ref()));
    progress.pulse();
    progress_box.append(&progress);

    content.append(&progress_box);
    content.append(&state);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    dialog.set_child(Some(&toolbar));
    dialog.present(Some(parent));
    (dialog, state, progress)
}

fn update_export_progress(
    state: &gtk::Label,
    progress: &gtk::ProgressBar,
    event: video::ExportProgress,
) {
    let event = shrimply_export_core::video::progress::presentation(event);
    state.set_label(tr!(event.label).as_ref());
    progress.set_fraction(event.fraction.unwrap_or(1.0));
    let arguments = event.arguments.into_iter().collect::<Vec<_>>();
    progress.set_text(Some(&shrimply_components_gtk::i18n::text_args(
        event.message,
        &arguments,
    )));
}

fn show_export_error(parent: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", tr!("Close").as_ref());
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog.choose(
        Some(parent.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        |_| {},
    );
}
