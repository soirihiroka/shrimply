use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_gtk_components::project_settings::ProjectSettingsSelector;
use shrimply_gtk_components::tr;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_WIDTH: i32 = 760;
const DEFAULT_HEIGHT: i32 = 560;
const RECENT_ROW_HEIGHT: i32 = 64;
const PROJECT_INFO_WIDTH: i32 = 500;
const PROJECT_PATH_LINES: i32 = 3;

fn main() -> glib::ExitCode {
    shrimply_support::diagnostics::init();
    shrimply_gtk_components::i18n::init_system_locale();
    let mut args = std::env::args_os().skip(1);
    if let Some(path) = args.next() {
        if args.next().is_some() {
            eprintln!(
                "usage: shrimply [PROJECT.shrimp|PROJECT.json|TIMELINE.otio|PROJECT.kdenlive]"
            );
            return glib::ExitCode::FAILURE;
        }
        return match shrimply_cross_ui_core::launcher::launch_editor(Path::new(&path)).and_then(
            |mut editor| {
                editor
                    .wait()
                    .map_err(|error| format!("could not wait for editor: {error}"))
            },
        ) {
            Ok(status) if status.success() => glib::ExitCode::SUCCESS,
            Ok(status) => {
                eprintln!("editor exited with {status}");
                glib::ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("{error}");
                glib::ExitCode::FAILURE
            }
        };
    }

    let app = adw::Application::new(
        Some("dev.shrimply.Shrimply"),
        gio::ApplicationFlags::NON_UNIQUE,
    );
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    shrimply_gtk_components::icons::register_bundled();

    let left_header = adw::HeaderBar::new();
    left_header.set_show_end_title_buttons(false);
    left_header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let right_header = adw::HeaderBar::new();
    right_header.set_show_start_title_buttons(false);
    let app_title = gtk::Label::builder()
        .label(tr!("Shrimply").as_ref())
        .css_classes(["title"])
        .build();
    right_header.set_title_widget(Some(&app_title));
    let new_button = gtk::Button::builder()
        .label(tr!("Create Project").as_ref())
        .css_classes(["suggested-action", "pill"])
        .width_request(160)
        .build();
    let open_button = gtk::Button::builder()
        .label(tr!("Open Project").as_ref())
        .css_classes(["pill"])
        .width_request(160)
        .build();
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    actions.set_margin_top(12);
    actions.set_margin_start(12);
    actions.set_margin_end(12);
    actions.set_valign(gtk::Align::Start);
    new_button.set_halign(gtk::Align::Center);
    open_button.set_halign(gtk::Align::Center);
    actions.append(&new_button);
    actions.append(&open_button);
    let sidebar_toolbar = adw::ToolbarView::new();
    sidebar_toolbar.add_top_bar(&left_header);
    sidebar_toolbar.set_content(Some(&actions));
    let sidebar = adw::NavigationPage::builder()
        .title(tr!("Shrimply").as_ref())
        .child(&sidebar_toolbar)
        .build();

    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr!("Search history").as_ref())
        .hexpand(true)
        .build();
    let clear_history = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text(tr!("Clear History").as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let search_controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    search_controls.set_margin_top(12);
    search_controls.set_margin_bottom(12);
    search_controls.set_margin_start(12);
    search_controls.set_margin_end(12);
    search_controls.append(&search);
    search_controls.append(&clear_history);
    let recent_area = gtk::Box::new(gtk::Orientation::Vertical, 8);
    recent_area.set_vexpand(true);
    recent_area.set_margin_bottom(12);
    recent_area.set_margin_start(12);
    recent_area.set_margin_end(12);
    let recent_scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&recent_area)
        .build();
    let history = gtk::Box::new(gtk::Orientation::Vertical, 0);
    history.set_hexpand(true);
    history.append(&search_controls);
    history.append(&recent_scroller);
    let history_toolbar = adw::ToolbarView::new();
    history_toolbar.add_top_bar(&right_header);
    history_toolbar.set_content(Some(&history));
    let history_page = adw::NavigationPage::builder()
        .title(tr!("History").as_ref())
        .child(&history_toolbar)
        .build();
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar)
        .content(&history_page)
        .min_sidebar_width(200.0)
        .max_sidebar_width(200.0)
        .sidebar_width_unit(adw::LengthUnit::Px)
        .collapsed(false)
        .build();

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr!("Shrimply").as_ref())
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .content(&split)
        .build();

    refresh_recents(&recent_area, &search, &window, app);
    search.connect_search_changed({
        let recent_area = recent_area.clone();
        let window = window.clone();
        let app = app.clone();
        move |search| refresh_recents(&recent_area, search, &window, &app)
    });
    clear_history.connect_clicked({
        let recent_area = recent_area.clone();
        let search = search.clone();
        let window = window.clone();
        let app = app.clone();
        move |_| {
            if let Err(error) = shrimply_cross_ui_core::launcher::clear_recent_projects() {
                show_error(&window, "Could not clear recent projects", &error);
            }
            refresh_recents(&recent_area, &search, &window, &app);
        }
    });
    new_button.connect_clicked({
        let window = window.clone();
        let app = app.clone();
        move |_| show_create_project(&window, &app)
    });
    open_button.connect_clicked({
        let window = window.clone();
        let app = app.clone();
        move |_| show_open_project(&window, &app)
    });
    window.present();
}

fn refresh_recents(
    area: &gtk::Box,
    search: &gtk::SearchEntry,
    window: &adw::ApplicationWindow,
    app: &adw::Application,
) {
    while let Some(child) = area.first_child() {
        area.remove(&child);
    }

    let projects = match shrimply_cross_ui_core::launcher::load_recent_projects(&search.text()) {
        Ok(projects) => projects,
        Err(error) => {
            tracing::warn!("Could not load recent projects: {error}");
            Vec::new()
        }
    };
    if projects.is_empty() {
        let empty = adw::StatusPage::builder()
            .icon_name("document-open-recent-symbolic")
            .title(
                tr!(if search.text().trim().is_empty() {
                    "No Recent Projects"
                } else {
                    "No Matching Projects"
                })
                .as_ref(),
            )
            .vexpand(true)
            .build();
        area.append(&empty);
        return;
    }

    for recent in projects {
        let last_edited = shrimply_cross_ui_core::launcher::last_edited(&recent.path);
        let last_edited_subtitle = last_edited
            .as_ref()
            .map(|date| {
                shrimply_gtk_components::i18n::text_args(
                    "Last edited %{date}",
                    &[("date", date.clone())],
                )
            })
            .unwrap_or_else(|| tr!("Last edited time unavailable").into_owned());
        let row = adw::ActionRow::builder()
            .title(&recent.name)
            .subtitle(&last_edited_subtitle)
            .activatable(true)
            .height_request(RECENT_ROW_HEIGHT)
            .build();
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(tr!("Project options").as_ref())
            .valign(gtk::Align::Center)
            .has_frame(false)
            .build();
        let popover = gtk::Popover::new();
        let options = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let info = gtk::Button::with_label(tr!("Info").as_ref());
        info.set_has_frame(false);
        let show_in_files = gtk::Button::with_label(tr!("Show in Files").as_ref());
        show_in_files.set_has_frame(false);
        let delete = gtk::Button::with_label(tr!("Delete").as_ref());
        delete.set_has_frame(false);
        options.append(&info);
        options.append(&show_in_files);
        options.append(&delete);
        popover.set_child(Some(&options));
        menu.set_popover(Some(&popover));
        row.add_suffix(&menu);
        row.connect_activated({
            let path = recent.path.clone();
            let window = window.clone();
            let app = app.clone();
            move |_| open_in_editor(&window, &app, &path)
        });
        info.connect_clicked({
            let name = recent.name.clone();
            let path = recent.path.clone();
            let window = window.clone();
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                show_project_info(&window, &name, &path, last_edited.as_deref());
            }
        });
        show_in_files.connect_clicked({
            let path = recent.path.clone();
            let window = window.clone();
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                if let Err(error) = shrimply_gtk_components::desktop_open::show_path_in_folder(
                    window.upcast_ref(),
                    path.clone(),
                ) {
                    show_error(&window, "Could not show project file", &error);
                }
            }
        });
        delete.connect_clicked({
            let path = recent.path;
            let area = area.clone();
            let search = search.clone();
            let window = window.clone();
            let app = app.clone();
            let popover = popover.clone();
            move |_| {
                popover.popdown();
                if let Err(error) = shrimply_cross_ui_core::launcher::remove_recent_project(&path) {
                    show_error(&window, "Could not remove recent project", &error);
                }
                refresh_recents(&area, &search, &window, &app);
            }
        });
        let card = adw::PreferencesGroup::new();
        card.add(&row);
        area.append(&card);
    }
}

fn show_project_info(
    window: &adw::ApplicationWindow,
    name: &str,
    path: &Path,
    last_edited: Option<&str>,
) {
    let unavailable = tr!("Unavailable");
    let details = adw::PreferencesGroup::new();
    details.add(
        &adw::ActionRow::builder()
            .title(tr!("Last Edited").as_ref())
            .subtitle(last_edited.unwrap_or(unavailable.as_ref()))
            .build(),
    );

    let location = adw::ActionRow::builder()
        .title(tr!("File Location").as_ref())
        .subtitle(path.to_string_lossy())
        .subtitle_lines(PROJECT_PATH_LINES)
        .build();
    let show_in_files = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(tr!("Show in Files").as_ref())
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    show_in_files.connect_clicked({
        let path = path.to_path_buf();
        let window = window.clone();
        move |button| {
            if let Err(error) = shrimply_gtk_components::desktop_open::show_path_in_folder(
                button.upcast_ref(),
                path.clone(),
            ) {
                show_error(&window, "Could not show project file", &error);
            }
        }
    });
    location.add_suffix(&show_in_files);
    location.set_activatable_widget(Some(&show_in_files));
    details.add(&location);

    let page = adw::PreferencesPage::new();
    page.add(&details);
    let dialog = adw::PreferencesDialog::builder()
        .title(name)
        .search_enabled(false)
        .content_width(PROJECT_INFO_WIDTH)
        .build();
    dialog.add(&page);
    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
}

fn show_open_project(window: &adw::ApplicationWindow, app: &adw::Application) {
    let window = window.clone();
    let app = app.clone();
    shrimply_gtk_components::project_open::open_project(&window.clone(), move |result| {
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                show_error(&window, "Could not open project", &error);
                return;
            }
        };
        open_in_editor(&window, &app, &path);
    });
}

fn show_create_project(window: &adw::ApplicationWindow, app: &adw::Application) {
    let selector = ProjectSettingsSelector::new();
    let preset = selector.preset.clone();
    let name = adw::EntryRow::builder()
        .title(tr!("Project Name").as_ref())
        .text(tr!("Untitled Project").as_ref())
        .build();
    let width = selector.width.clone();
    let height = selector.height.clone();
    let fps = selector.fps.clone();
    let names = adw::PreferencesGroup::new();
    names.add(&name);
    let presets = adw::PreferencesGroup::new();
    presets.add(&preset);
    let settings = adw::PreferencesGroup::builder()
        .title(tr!("Project Settings").as_ref())
        .build();
    settings.add(&width);
    settings.add(&height);
    settings.add(&fps);
    let create = adw::ButtonRow::builder()
        .title(tr!("Create Project").as_ref())
        .build();
    create.add_css_class("suggested-action");
    let actions = adw::PreferencesGroup::new();
    actions.add(&create);
    let page = adw::PreferencesPage::new();
    page.add(&names);
    page.add(&presets);
    page.add(&settings);
    page.add(&actions);
    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Create Project").as_ref())
        .search_enabled(false)
        .build();
    dialog.add(&page);

    name.connect_changed({
        let create = create.clone();
        move |row| create.set_sensitive(!row.text().trim().is_empty())
    });
    create.connect_activated({
        let dialog = dialog.clone();
        let window = window.clone();
        let app = app.clone();
        move |_| {
            let project_name = name.text().trim().to_string();
            let Some((canvas_size, fps)) = selector.settings() else {
                show_error(&window, "Could not create project", "Invalid frame rate.");
                return;
            };
            let filter = shrimply_gtk_components::project_open::project_file_filter();
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let label = "Create Project";
            let save = gtk::FileDialog::builder()
                .title(tr!(label).as_ref())
                .initial_name(shrimply_cross_ui_core::launcher::default_project_filename(
                    &project_name,
                ))
                .filters(&filters)
                .default_filter(&filter)
                .build();
            dialog.close();
            let window_for_save = window.clone();
            let app_for_save = app.clone();
            shrimply_gtk_components::file_picker::save(
                label,
                &save,
                Some(window.upcast_ref::<gtk::Window>()),
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    let Some(path) = file.path() else {
                        show_error(
                            &window_for_save,
                            "Could not create project",
                            "The selected location does not have a local path.",
                        );
                        return;
                    };
                    let path = match shrimply_cross_ui_core::launcher::create_project(
                        path,
                        &project_name,
                        canvas_size,
                        fps,
                    ) {
                        Ok(path) => path,
                        Err(error) => {
                            show_error(&window_for_save, "Could not create project", &error);
                            return;
                        }
                    };
                    open_in_editor(&window_for_save, &app_for_save, &path);
                },
            );
        }
    });
    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
}

fn open_in_editor(window: &adw::ApplicationWindow, app: &adw::Application, path: &Path) {
    match shrimply_cross_ui_core::launcher::launch_editor(path) {
        Ok(mut editor) => {
            let mut hold = Some(app.hold());
            window.set_visible(false);
            let (sender, receiver) = mpsc::channel();
            thread::spawn(move || {
                let _ = sender.send(editor.wait());
            });
            let app = app.clone();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                match receiver.try_recv() {
                    Ok(Ok(status)) if status.success() => {
                        hold.take();
                        app.quit();
                        glib::ControlFlow::Break
                    }
                    Ok(Ok(status)) => {
                        eprintln!("editor exited with {status}");
                        std::process::exit(status.code().unwrap_or(1).clamp(1, 255));
                    }
                    Ok(Err(error)) => {
                        eprintln!("could not wait for editor: {error}");
                        std::process::exit(1);
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        eprintln!("editor status channel disconnected");
                        std::process::exit(1);
                    }
                    Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                }
            });
        }
        Err(error) => show_error(window, "Could not start editor", &error),
    }
}

fn show_error(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", tr!("Close").as_ref());
    dialog.set_close_response("close");
    dialog.present(Some(window));
}
