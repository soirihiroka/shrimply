use adw::prelude::*;
use gtk::{gio, glib};
use shrimply_components_gtk::project_settings::ProjectSettingsSelector;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::{
    SingleLineTextInput, adaptive_menu, control_row, dropdown, menu_action,
};
use shrimply_cross_ui_core::launcher::{self, Launcher, ProjectLocation};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_WIDTH: i32 = 760;
const DEFAULT_HEIGHT: i32 = 560;
const PROJECT_PAGE_SIZE: usize = 40;
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
    match launcher::project_path_argument() {
        Ok(Some(path)) => return shrimply_editor_gtk::run(path),
        Ok(None) => (),
        Err(error) => {
            eprintln!("{error}");
            return glib::ExitCode::FAILURE;
        }
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

    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr!("Search Projects").as_ref())
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .activate_on_single_click(true)
        .css_classes(["boxed-list"])
        .build();
    let hint = gtk::Label::new(Some(tr!("No Projects").as_ref()));
    list.set_placeholder(Some(&hint));
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let projects = gtk::Box::new(gtk::Orientation::Vertical, 12);
    projects.set_margin_start(12);
    projects.set_margin_end(12);
    projects.set_margin_bottom(12);
    projects.append(&search);
    projects.append(&scroller);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_start_title_buttons(false);
    let menu = gio::Menu::new();
    for (label, action) in [
        ("Settings", "win.preferences"),
        ("Project Servers", "win.project-servers"),
        ("About Shrimply", "win.about"),
    ] {
        menu.append(Some(tr!(label).as_ref()), Some(action));
    }
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(tr!("Main Menu").as_ref())
        .menu_model(&menu)
        .has_frame(false)
        .build();
    header.pack_end(&menu_button);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&projects));
    let projects = adw::NavigationPage::builder()
        .title(tr!("Projects").as_ref())
        .child(&toolbar)
        .build();

    let create = gtk::Button::with_label(tr!("Create Project").as_ref());
    create.add_css_class("suggested-action");
    create.add_css_class("pill");
    let open = gtk::Button::with_label(tr!("Open Project").as_ref());
    open.add_css_class("pill");
    let actions = gtk::Box::new(gtk::Orientation::Vertical, 12);
    actions.set_halign(gtk::Align::Center);
    actions.set_valign(gtk::Align::Center);
    actions.append(&create);
    actions.append(&open);
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&actions));
    let actions = adw::NavigationPage::builder()
        .title(tr!("Shrimply").as_ref())
        .child(&toolbar)
        .build();
    let split = adw::NavigationSplitView::builder()
        .sidebar(&actions)
        .content(&projects)
        .min_sidebar_width(200.0)
        .max_sidebar_width(200.0)
        .sidebar_width_unit(adw::LengthUnit::Px)
        .build();
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr!("Shrimply").as_ref())
        .default_width(DEFAULT_WIDTH)
        .default_height(DEFAULT_HEIGHT)
        .content(&split)
        .build();
    adaptive_menu(&menu_button, "win", &window);

    let model = Rc::new(RefCell::new(Launcher::default()));
    let rebuild = {
        let (list, search, hint, model) =
            (list.clone(), search.clone(), hint.clone(), model.clone());
        move || {
            model.borrow_mut().set_search(&search.text());
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            hint.set_label(
                tr!(if search.text().trim().is_empty() {
                    "No Projects"
                } else {
                    "No Matching Projects"
                })
                .as_ref(),
            );
            append_projects(&list, &model.borrow(), 0);
        }
    };
    let reload = {
        let (model, rebuild, window, hint) = (
            model.clone(),
            rebuild.clone(),
            window.downgrade(),
            hint.clone(),
        );
        move || {
            let (model, rebuild, window) = (model.clone(), rebuild.clone(), window.clone());
            hint.set_label(tr!("Loading Projects").as_ref());
            let load = model.borrow_mut().begin_project_load();
            glib::spawn_future_local(async move {
                let projects = gio::spawn_blocking(launcher::load_projects)
                    .await
                    .expect("project catalog worker");
                let result = model.borrow_mut().finish_project_load(load, projects);
                match result {
                    Ok(true) => rebuild(),
                    Ok(false) => (),
                    Err(error) => {
                        if let Some(window) = window.upgrade() {
                            show_error(&window, "Could not load projects", &error);
                        }
                    }
                }
            });
        }
    };
    search.connect_search_changed(move |_| rebuild());
    scroller.connect_edge_reached({
        let (list, model) = (list.clone(), model.clone());
        move |_, edge| {
            if edge == gtk::PositionType::Bottom {
                append_projects(
                    &list,
                    &model.borrow(),
                    list.observe_children().n_items() as usize,
                );
            }
        }
    });
    list.connect_row_activated({
        let model = model.clone();
        let (window, app) = (window.clone(), app.clone());
        move |_, row| {
            if let Some(entry) = model.borrow().project(row.index() as usize).cloned() {
                open_remote(&window, &app, "Could not open project", move || {
                    entry.open(None)
                });
            }
        }
    });
    create.connect_clicked({
        let (window, app) = (window.clone(), app.clone());
        move |_| show_create_project(&window, &app)
    });
    open.connect_clicked({
        let (window, app) = (window.clone(), app.clone());
        move |_| show_open_project(&window, &app)
    });
    match launcher::local_projects() {
        Ok(projects) => {
            model.borrow_mut().set_projects(projects);
            append_projects(&list, &model.borrow(), 0);
        }
        Err(error) => show_error(&window, "Could not load projects", &error),
    }
    reload();
    window.add_action(&menu_action("project-servers", None, {
        let window = window.downgrade();
        let reload = reload.clone();
        move || {
            let window = window.upgrade()?;
            let reload = reload.clone();
            shrimply_preferences_gtk::page::show_project_servers(&window)
                .connect_closed(move |_| reload());
            None
        }
    }));
    window.add_action(&menu_action("preferences", None, {
        let window = window.downgrade();
        move || {
            shrimply_preferences_gtk::page::show_preferences_dialog(
                &window.upgrade()?,
                shrimply_preferences_gtk::store::open_with_defaults(),
            );
            None
        }
    }));
    window.add_action(&menu_action("about", None, {
        let window = window.downgrade();
        move || {
            shrimply_components_gtk::about::show(&window.upgrade()?, env!("CARGO_PKG_VERSION"));
            None
        }
    }));
    for name in ["info", "show", "restart", "delete"] {
        let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
        let (weak_window, model, reload) = (window.downgrade(), model.clone(), reload.clone());
        action.connect_activate(move |_, value| {
            let Some((window, key)) = weak_window
                .upgrade()
                .zip(value.and_then(|value| value.get::<String>()))
            else {
                return;
            };
            let Some(project) = model.borrow().project_by_key(&key).cloned() else {
                return;
            };
            let reload = reload.clone();
            glib::spawn_future_local(async move {
                match name {
                    "info" => {
                        show_error(&window, &project.name, &launcher::project_details(&project));
                    }
                    "show" => {
                        if let Some(path) = project.path
                            && let Err(error) =
                                shrimply_components_gtk::desktop_open::show_path_in_folder(
                                    window.upcast_ref(),
                                    path,
                                )
                        {
                            show_error(&window, "Could not show project in Files", &error);
                        }
                    }
                    "restart" => {
                        if let Some(path) = project.path
                            && let Err(error) =
                                gio::spawn_blocking(move || launcher::restart_local_service(&path))
                                    .await
                                    .expect("restart worker")
                        {
                            show_error(&window, "Could not restart the project service", &error);
                        }
                    }
                    "delete" => {
                        let message = if project.remote.is_some() {
                            "Delete this project from the server for everyone?"
                        } else {
                            "Permanently delete this project file?"
                        };
                        let dialog =
                            adw::AlertDialog::new(Some(&project.name), Some(tr!(message).as_ref()));
                        dialog.add_response("cancel", tr!("Cancel").as_ref());
                        dialog.add_response("delete", tr!("Delete Project").as_ref());
                        dialog.set_response_appearance(
                            "delete",
                            adw::ResponseAppearance::Destructive,
                        );
                        dialog.set_close_response("cancel");
                        if dialog.choose_future(Some(&window)).await == "delete" {
                            if let Err(error) =
                                gio::spawn_blocking(move || launcher::delete_project(&project))
                                    .await
                                    .expect("delete worker")
                            {
                                show_error(&window, "Could not delete project", &error);
                            }
                            reload();
                        }
                    }
                    _ => unreachable!(),
                }
            });
        });
        window.add_action(&action);
    }
    window.present();
}

fn append_projects(list: &gtk::ListBox, model: &Launcher, start: usize) {
    for index in start..model.project_count().min(start + PROJECT_PAGE_SIZE) {
        let project = model.project(index).expect("visible project");
        let row = adw::ActionRow::builder()
            .title(&project.name)
            .use_markup(false)
            .activatable(true)
            .build();
        let icon = gtk::Image::from_icon_name(if project.remote.is_some() {
            "folder-remote-symbolic"
        } else {
            "folder-symbolic"
        });
        row.add_prefix(&icon);
        let subtitle = [
            launcher::project_access_label(project),
            project.remote.as_ref().map(|remote| remote.server.clone()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        row.set_subtitle(&subtitle);
        let menu = gio::Menu::new();
        for (visible, label, action) in [
            (true, "Project Information", "win.info"),
            (project.path.is_some(), "Show in Files", "win.show"),
            (
                project.path.is_some(),
                "Restart Local Service",
                "win.restart",
            ),
            (true, "Delete Project…", "win.delete"),
        ] {
            if visible {
                let item = gio::MenuItem::new(Some(tr!(label).as_ref()), None);
                item.set_action_and_target_value(Some(action), Some(&project.key().to_variant()));
                menu.append_item(&item);
            }
        }
        let options = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text(tr!("Project options").as_ref())
            .menu_model(&menu)
            .has_frame(false)
            .valign(gtk::Align::Center)
            .build();
        if let Some(window) = list.root().and_downcast::<adw::ApplicationWindow>() {
            adaptive_menu(&options, "win", &window);
        }
        row.add_suffix(&options);
        list.append(&row);
    }
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

fn open_remote(
    window: &adw::ApplicationWindow,
    app: &adw::Application,
    heading: &'static str,
    operation: impl FnOnce() -> Result<PathBuf, String> + Send + 'static,
) {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(operation());
    });
    let window = window.clone();
    let app = app.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receive.try_recv() {
            Ok(Ok(path)) => open_in_editor(&window, &app, &path),
            Ok(Err(error)) => show_error(&window, heading, &error),
            Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
            Err(mpsc::TryRecvError::Disconnected) => {
                show_error(&window, heading, "The project worker stopped unexpectedly.")
            }
        }
        glib::ControlFlow::Break
    });
}

fn show_create_project(window: &adw::ApplicationWindow, app: &adw::Application) {
    let selector = ProjectSettingsSelector::new();
    let name = SingleLineTextInput::builder(tr!("Untitled Project").as_ref()).build();
    let servers = launcher::project_servers();
    let destination = dropdown(
        0,
        std::iter::once("Local".to_owned())
            .chain(servers.iter().cloned())
            .enumerate(),
        |_| {},
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&control_row("Location", &destination));
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
            if let Some(server) = destination
                .selected()
                .checked_sub(1)
                .and_then(|index| servers.get(index as usize))
                .cloned()
            {
                dialog.close();
                open_remote(&window, &app, "Could not create project", move || {
                    launcher::create_project(
                        ProjectLocation::Remote(server),
                        &project_name,
                        (canvas_size, fps),
                        &project_tags,
                    )
                });
                return;
            }
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
                    let path = match launcher::create_project(
                        ProjectLocation::Local(path),
                        &project_name,
                        (canvas_size, fps),
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
