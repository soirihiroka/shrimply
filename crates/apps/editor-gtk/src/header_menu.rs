use crate::{export, player_state, project};
use adw::prelude::*;
use gtk::gio;
use shrimply_gtk_components::tr;
use shrimply_gtk_components::ui::I18nMenuExt;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

pub(crate) fn add(
    header_bar: &adw::HeaderBar,
    window: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    session: Rc<shrimply_cross_ui_core::editor::EditorSession>,
) {
    let project = session.project.clone();
    let player_state = session.player_state.clone();
    let preferences = session.preferences.clone();
    let menu = gio::Menu::new();
    let project_menu = gio::Menu::new();
    project_menu.append_i18n("New Project", "win.new-project");
    project_menu.append_i18n("Open Project…", "win.open-project");
    let track_menu = gio::Menu::new();
    track_menu.append_i18n("Caption Track", "win.new-caption-track");
    track_menu.append_i18n("Video Track", "win.new-video-track");
    track_menu.append_i18n("Audio Track", "win.new-audio-track");
    project_menu.append_submenu_i18n("New Track", &track_menu);
    project_menu.append_i18n("Save", "win.save");
    project_menu.append_i18n("Save As…", "win.save-as");
    menu.append_section(None, &project_menu);

    let history_menu = gio::Menu::new();
    history_menu.append_i18n("Undo", "win.undo");
    history_menu.append_i18n("Redo", "win.redo");
    menu.append_section(None, &history_menu);

    let settings_menu = gio::Menu::new();
    settings_menu.append_i18n("Preferences", "win.preferences");
    settings_menu.append_i18n("Keyboard Shortcuts", "win.show-shortcuts");
    menu.append_section(None, &settings_menu);

    let about_menu = gio::Menu::new();
    about_menu.append_i18n("About Shrimply", "win.about");
    menu.append_section(None, &about_menu);

    add_action(window, "new-project", {
        let window = window.clone();
        move || {
            if let Err(error) = launch_sibling("shrimply", None) {
                show_error_dialog(&window, "Could not create project", &error);
            }
        }
    });
    add_action(window, "open-project", {
        let window = window.clone();
        move || show_open_project_dialog(&window)
    });
    for (name, kind) in [
        ("new-caption-track", NewTrackKind::Caption),
        ("new-video-track", NewTrackKind::Video),
        ("new-audio-track", NewTrackKind::Audio),
    ] {
        add_action(window, name, {
            let project = project.clone();
            let player_state = player_state.clone();
            move || add_track(&project, &player_state, kind)
        });
    }
    add_action(window, "save-as", {
        let window = window.clone();
        let toasts = toasts.clone();
        let session = session.clone();
        move || show_save_as_dialog(&window, &toasts, &session)
    });
    add_action(window, "save", {
        let window = window.clone();
        let session = session.clone();
        move || {
            if let Err(error) = session.save() {
                show_error_dialog(&window, "Could not save project", &error);
            }
        }
    });
    add_action(window, "undo", {
        let project = project.clone();
        let player_state = player_state.clone();
        move || change_history(&project, &player_state, project::undo)
    });
    add_action(window, "redo", {
        let project = project.clone();
        let player_state = player_state.clone();
        move || change_history(&project, &player_state, project::redo)
    });
    add_action(window, "show-shortcuts", {
        let window = window.clone();
        move || show_shortcuts_dialog(&window)
    });
    add_action(window, "preferences", {
        let window = window.clone();
        let preferences = preferences.clone();
        move || {
            shrimply_preferences_gtk::page::show_preferences_dialog(&window, preferences.clone())
        }
    });
    add_action(window, "about", {
        let window = window.clone();
        move || show_about_dialog(&window)
    });

    if let Some(app) = window.application() {
        for (action, accelerators) in [
            ("win.new-project", &["<Primary>n"][..]),
            ("win.open-project", &["<Primary>o"][..]),
            ("win.save", &["<Primary>s"][..]),
            ("win.save-as", &["<Primary><Shift>s"][..]),
            ("win.undo", &["<Primary>z"][..]),
            ("win.redo", &["<Primary><Shift>z", "<Primary>y"][..]),
            ("win.preferences", &["<Primary>comma"][..]),
            ("win.show-shortcuts", &["<Primary>question"][..]),
        ] {
            app.set_accels_for_action(action, accelerators);
        }
    }

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .has_frame(false)
        .popover(&popover)
        .build();
    let export_button = export::build_export_button(window, toasts, project, preferences);
    let header_right = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header_right.append(&export_button);
    header_right.append(&menu_button);
    header_bar.pack_end(&header_right);
}

fn add_action<F>(window: &adw::ApplicationWindow, name: &str, activate: F)
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    window.add_action(&action);
}

fn show_shortcuts_dialog(window: &adw::ApplicationWindow) {
    let shortcuts = adw::ShortcutsDialog::builder()
        .title(tr!("Keyboard Shortcuts").as_ref())
        .build();
    let section = adw::ShortcutsSection::new(Some(tr!("General").as_ref()));
    section.add(adw::ShortcutsItem::new(
        tr!("New Project").as_ref(),
        "Ctrl+N",
    ));
    section.add(adw::ShortcutsItem::new(
        tr!("Open Project").as_ref(),
        "Ctrl+O",
    ));
    section.add(adw::ShortcutsItem::new(tr!("Save").as_ref(), "Ctrl+S"));
    section.add(adw::ShortcutsItem::new(
        tr!("Save As").as_ref(),
        "Ctrl+Shift+S",
    ));
    section.add(adw::ShortcutsItem::new(tr!("Undo").as_ref(), "Ctrl+Z"));
    section.add(adw::ShortcutsItem::new(
        tr!("Redo").as_ref(),
        "Ctrl+Shift+Z",
    ));
    section.add(adw::ShortcutsItem::new(
        tr!("Preferences").as_ref(),
        "Ctrl+,",
    ));
    section.add(adw::ShortcutsItem::new(
        tr!("Show Keyboard Shortcuts").as_ref(),
        "Ctrl+?",
    ));
    section.add(adw::ShortcutsItem::new(
        tr!("Play / Pause").as_ref(),
        "Space",
    ));
    section.add(adw::ShortcutsItem::new(tr!("Step playback").as_ref(), "L"));
    shortcuts.add(section);
    let timeline = adw::ShortcutsSection::new(Some(tr!("Timeline").as_ref()));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Split all clips at playhead").as_ref(),
        "S",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Split all clips and select left").as_ref(),
        "Shift+S",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Ripple trim selected clip to playhead").as_ref(),
        "Q",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Delete selection").as_ref(),
        "D",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Ripple cut").as_ref(),
        "Shift+D",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Cut selection").as_ref(),
        "Ctrl+X",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Replace selected item properties").as_ref(),
        "Ctrl+Shift+V",
    ));
    timeline.add(adw::ShortcutsItem::new(
        tr!("Toggle timeline zoom").as_ref(),
        "Z",
    ));
    shortcuts.add(timeline);
    shortcuts.present(Some(window));
}

fn show_open_project_dialog(window: &adw::ApplicationWindow) {
    let window = window.clone();
    shrimply_gtk_components::project_open::open_project(&window.clone(), move |result| {
        let path = match result {
            Ok(Some(path)) => path,
            Ok(None) => return,
            Err(error) => {
                show_error_dialog(&window, "Could not open project", &error);
                return;
            }
        };
        if let Err(error) = launch_sibling("shrimply-editor", Some(&path)) {
            show_error_dialog(&window, "Could not open project", &error);
        }
    });
}

fn show_save_as_dialog(
    window: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    session: &Rc<shrimply_cross_ui_core::editor::EditorSession>,
) {
    let label = "Save Project As";
    let filter = shrimply_gtk_components::project_open::project_file_filter();
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let initial_name = shrimply_cross_ui_core::editor::suggested_save_as_path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("save-as suggestion must have a file name")
        .to_string();
    let dialog = gtk::FileDialog::builder()
        .title(tr!(label).as_ref())
        .initial_name(initial_name)
        .filters(&filters)
        .default_filter(&filter)
        .build();
    let window = window.clone();
    let parent = window.clone();
    let toasts = toasts.clone();
    let session = session.clone();
    shrimply_gtk_components::file_picker::save(
        label,
        &dialog,
        Some(parent.upcast_ref::<gtk::Window>()),
        move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(path) = file.path() else {
                show_error_dialog(
                    &window,
                    "Could not save project",
                    "The selected location does not have a local path.",
                );
                return;
            };
            if let Err(error) = session.save_as(path) {
                show_error_dialog(&window, "Could not save project", &error);
                return;
            }
            shrimply_gtk_components::toast::show_confirmation(
                &toasts,
                "Project saved to the new location",
            );
        },
    );
}

fn launch_sibling(name: &str, argument: Option<&Path>) -> Result<(), String> {
    let sibling = std::env::current_exe()
        .map(|path| path.with_file_name(name))
        .ok()
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name));
    let mut command = Command::new(&sibling);
    if let Some(argument) = argument {
        command.arg(argument);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not launch {}: {error}", sibling.display()))
}

fn show_error_dialog(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    adw::AlertDialog::new(Some(heading), Some(body)).present(Some(window));
}

fn show_about_dialog(window: &adw::ApplicationWindow) {
    use shrimply_component_core::about;
    let dialog = adw::AboutDialog::builder()
        .application_name(about::NAME)
        .application_icon(about::ICON_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(about::DESCRIPTION)
        .developer_name(about::DEVELOPER)
        .developers([about::DEVELOPER])
        .website(about::WEBSITE)
        .issue_url(about::ISSUE_URL)
        .license_type(gtk::License::Gpl30)
        .title(tr!("About Shrimply").as_ref())
        .build();
    dialog.add_credit_section(Some(about::CREDIT_HEADING), about::CREDITS);
    dialog.present(Some(window));
}

#[derive(Clone, Copy)]
enum NewTrackKind {
    Caption,
    Video,
    Audio,
}

fn add_track(
    project: &Rc<RefCell<project::Project>>,
    player_state: &player_state::SharedPlayerState,
    kind: NewTrackKind,
) {
    {
        let mut project = project.borrow_mut();
        match kind {
            NewTrackKind::Caption => project.caption_tracks.push(Default::default()),
            NewTrackKind::Video => project.video_tracks.push(Default::default()),
            NewTrackKind::Audio => project.audio_tracks.push(Default::default()),
        }
        project::commit_edit(&project, "create-timeline-track");
    }
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            audio: matches!(kind, NewTrackKind::Audio),
            video: matches!(kind, NewTrackKind::Video),
            captions: matches!(kind, NewTrackKind::Caption),
            inspector: true,
            ..Default::default()
        },
    );
}

fn change_history(
    project: &Rc<RefCell<project::Project>>,
    player_state: &player_state::SharedPlayerState,
    change: fn(&mut project::Project) -> bool,
) {
    if !change(&mut project.borrow_mut()) {
        return;
    }
    let (duration, frame_rate) = {
        let project = project.borrow();
        (project.duration(), project.fps)
    };
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            duration: Some(duration),
            frame_rate: Some(frame_rate),
            audio: true,
            audio_beats: true,
            audio_waveforms: true,
            video: true,
            live_preview: false,
            captions: true,
            inspector: true,
        },
    );
}
