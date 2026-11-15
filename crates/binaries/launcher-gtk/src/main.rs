use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_components_gtk::presentation::{self, RecentProjectAction};
use shrimply_components_gtk::project_settings::ProjectSettingsSelector;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::{SingleLineTextInput, control_row};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_WIDTH: i32 = 760;
const DEFAULT_HEIGHT: i32 = 560;
const RECENT_ROW_HEIGHT: i32 = 64;
const PROJECT_INFO_WIDTH: i32 = 500;
const PROJECT_PATH_LINES: i32 = 3;
const CREATE_PROJECT_WIDTH: i32 = 520;

fn main() -> glib::ExitCode {
    if let Some(status) = shrimply_cli::run_if_requested() {
        return glib::ExitCode::from(status);
    }
    shrimply_process_reporting::diagnostics::init();
    shrimply_components_gtk::i18n::init_system_locale();
    // Linux selects and validates its GPU backend in the editor.
    #[cfg(target_os = "windows")]
    if let Err(error) = shrimply_cross_ui_core::launcher::check_nvidia_gpu() {
        tracing::error!("{error}");
        if let Err(error) = adw::init() {
            eprintln!("could not initialize GTK: {error}");
            return glib::ExitCode::FAILURE;
        }
        let dialog = adw::AlertDialog::builder()
            .heading(tr!(shrimply_cross_ui_core::launcher::UNSUPPORTED_GPU_HEADING).as_ref())
            .body(tr!(shrimply_cross_ui_core::launcher::UNSUPPORTED_GPU_MESSAGE).as_ref())
            .build();
        dialog.add_response("close", tr!("Close").as_ref());
        dialog.set_close_response("close");
        glib::MainContext::default().block_on(dialog.choose_future(None::<&gtk::Widget>));
        return glib::ExitCode::FAILURE;
    }
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
    shrimply_components_gtk::icons::register_bundled();

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
        let last_edited = shrimply_cross_ui_core::launcher::recent_last_edited(&recent);
        let last_edited_subtitle = presentation::recent_project_subtitle(last_edited.as_deref());
        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .build();
        details.append(
            &gtk::Label::builder()
                .label(&recent.name)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        if !recent.tags.is_empty() {
            let tags = adw::WrapBox::builder()
                .child_spacing(6)
                .line_spacing(6)
                .build();
            for tag in &recent.tags {
                tags.append(&shrimply_components_gtk::ui::tag_pill(tag, false));
            }
            details.prepend(&tags);
        }
        details.append(
            &gtk::Label::builder()
                .label(&last_edited_subtitle)
                .xalign(0.0)
                .css_classes(["dim-label", "caption"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&details);
        let row = adw::PreferencesRow::builder()
            .title(&recent.name)
            .use_markup(false)
            .child(&content)
            .activatable(true)
            .height_request(RECENT_ROW_HEIGHT)
            .build();
        let menu = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(tr!(presentation::RECENT_PROJECT_MENU_LABEL).as_ref())
            .valign(gtk::Align::Center)
            .has_frame(false)
            .build();
        let model = gio::Menu::new();
        model.append(
            Some(&RecentProjectAction::Info.label()),
            Some("recent.info"),
        );
        model.append(
            Some(&RecentProjectAction::ShowInFiles.label()),
            Some("recent.show"),
        );
        model.append(
            Some(&RecentProjectAction::Delete.label()),
            Some("recent.delete"),
        );
        let actions = gio::SimpleActionGroup::new();
        menu.set_menu_model(Some(&model));
        menu.insert_action_group("recent", Some(&actions));
        shrimply_components_gtk::ui::adaptive_menu(&menu, "recent", &actions);
        content.append(&menu);
        let card = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .activate_on_single_click(true)
            .css_classes(["boxed-list"])
            .build();
        card.append(&row);
        card.connect_row_activated({
            let path = recent.path.clone();
            let window = window.clone();
            let app = app.clone();
            move |_, _| open_in_editor(&window, &app, &path)
        });
        actions.add_action(&shrimply_components_gtk::ui::menu_action("info", None, {
            let name = recent.name.clone();
            let path = recent.path.clone();
            let window = window.downgrade();
            move || {
                let window = window.upgrade()?;
                show_project_info(&window, &name, &path, last_edited.as_deref());
                None
            }
        }));
        actions.add_action(&shrimply_components_gtk::ui::menu_action("show", None, {
            let path = recent.path.clone();
            let window = window.downgrade();
            move || {
                let window = window.upgrade()?;
                if let Err(error) = shrimply_components_gtk::desktop_open::show_path_in_folder(
                    window.upcast_ref(),
                    path.clone(),
                ) {
                    show_error(&window, "Could not show project file", &error);
                }
                None
            }
        }));
        actions.add_action(&shrimply_components_gtk::ui::menu_action("delete", None, {
            let path = recent.path;
            let area = area.downgrade();
            let search = search.downgrade();
            let window = window.downgrade();
            let app = app.downgrade();
            move || {
                let (Some(area), Some(search), Some(window), Some(app)) = (
                    area.upgrade(),
                    search.upgrade(),
                    window.upgrade(),
                    app.upgrade(),
                ) else {
                    return None;
                };
                if let Err(error) = shrimply_cross_ui_core::launcher::remove_recent_project(&path) {
                    show_error(&window, "Could not remove recent project", &error);
                }
                refresh_recents(&area, &search, &window, &app);
                None
            }
        }));
        area.append(&card);
    }
}

fn show_project_info(
    window: &adw::ApplicationWindow,
    name: &str,
    path: &Path,
    last_edited: Option<&str>,
) {
    let info = presentation::recent_project_info(name, &path.to_string_lossy(), last_edited);
    let details = adw::PreferencesGroup::new();
    for field in info.fields {
        let row = adw::ActionRow::builder()
            .title(&field.label)
            .subtitle(&field.value)
            .build();
        if let Some(action) = field.action {
            row.set_subtitle_lines(PROJECT_PATH_LINES);
            let show_in_files = gtk::Button::builder()
                .icon_name(action.icon())
                .tooltip_text(action.label())
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            show_in_files.connect_clicked({
                let path = path.to_path_buf();
                let window = window.clone();
                move |button| {
                    if let Err(error) = shrimply_components_gtk::desktop_open::show_path_in_folder(
                        button.upcast_ref(),
                        path.clone(),
                    ) {
                        show_error(&window, "Could not show project file", &error);
                    }
                }
            });
            row.add_suffix(&show_in_files);
            row.set_activatable_widget(Some(&show_in_files));
        }
        details.add(&row);
    }

    let page = adw::PreferencesPage::new();
    page.add(&details);
    let dialog = adw::PreferencesDialog::builder()
        .title(&info.title)
        .search_enabled(false)
        .content_width(PROJECT_INFO_WIDTH)
        .build();
    dialog.add(&page);
    dialog.present(Some(window.upcast_ref::<gtk::Widget>()));
}

fn show_open_project(window: &adw::ApplicationWindow, app: &adw::Application) {
    let window = window.clone();
    let app = app.clone();
    shrimply_components_gtk::project_open::open_project(&window.clone(), move |result| {
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
    let name = SingleLineTextInput::builder(tr!("Untitled Project").as_ref()).build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&control_row("Project Name", &name));
    let tags = Rc::new(RefCell::new(Vec::new()));
    content.append(&shrimply_components_gtk::ui::tag_editor(Vec::new(), {
        let tags = tags.clone();
        move |next| *tags.borrow_mut() = next.to_vec()
    }));
    let names = adw::PreferencesGroup::new();
    names.add(&content);
    let create = adw::ButtonRow::builder()
        .title(tr!("Create Project").as_ref())
        .build();
    create.add_css_class("suggested-action");
    let actions = adw::PreferencesGroup::new();
    actions.add(&create);
    let page = adw::PreferencesPage::new();
    page.add(&names);
    page.add(&selector.group);
    page.add(&actions);
    let dialog = adw::PreferencesDialog::builder()
        .title(tr!("Create Project").as_ref())
        .search_enabled(false)
        .content_width(CREATE_PROJECT_WIDTH)
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
            let project_tags = tags.borrow().clone();
            let Some((canvas_size, fps)) = selector.settings() else {
                show_error(&window, "Could not create project", "Invalid frame rate.");
                return;
            };
            let filter = shrimply_components_gtk::project_open::project_file_filter();
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
            shrimply_components_gtk::file_picker::save(
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
                        &project_tags,
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
    tracing::info!(path = %path.display(), "Opening project from launcher");
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
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "Could not start editor");
            show_error(window, "Could not start editor", &error);
        }
    }
}

fn show_error(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", tr!("Close").as_ref());
    dialog.set_close_response("close");
    dialog.present(Some(window));
}
