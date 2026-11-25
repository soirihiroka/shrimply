use crate::{export, player_state, project};
use adw::prelude::*;
use gtk::gio;
use shrimply_components_gtk::tr;
use shrimply_components_gtk::ui::I18nMenuExt;
use shrimply_cross_ui_core::editor::{
    EditorSession,
    project_menu::{self, Action, Entry},
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

pub(crate) fn add(
    header_bar: &adw::HeaderBar,
    window: &adw::ApplicationWindow,
    toasts: &adw::ToastOverlay,
    session: Rc<EditorSession>,
) {
    let menu = gio::Menu::new();
    let sections = project_menu::sections(|action| host_action(action).is_some());
    let mut history = Vec::new();
    for section in sections {
        let group = gio::Menu::new();
        append_entries(&group, &section, &mut |action| {
            let command = add_action(window, action.id(), {
                let window = window.clone();
                let toasts = toasts.clone();
                let session = session.clone();
                move || match session.activate_project_menu(action) {
                    Ok(Some(action)) => host_action(action).expect("advertised window callback")(
                        &window, &toasts, &session,
                    ),
                    Ok(None) => {}
                    Err(error) => show_error_dialog(&window, action.label(), &error),
                }
            });
            if action.history().is_some() {
                command.set_enabled(action.enabled(project::can_undo(), project::can_redo()));
                history.push((action, command.downgrade()));
            }
        });
        menu.append_section(None, &group);
    }
    player_state::connect_named(
        &session.player_state,
        "project menu history",
        move |event| {
            if matches!(event, player_state::PlayerEvent::Project(_)) {
                let (can_undo, can_redo) = (project::can_undo(), project::can_redo());
                for (action, command) in &history {
                    if let Some(command) = command.upgrade() {
                        command.set_enabled(action.enabled(can_undo, can_redo));
                    }
                }
            }
        },
    );

    if let Some(app) = window.application() {
        for (action, accelerators) in [
            (Action::NewProject, &["<Primary>n"][..]),
            (Action::OpenProject, &["<Primary>o"][..]),
            (Action::Save, &["<Primary>s"][..]),
            (Action::SaveAs, &["<Primary><Shift>s"][..]),
            (Action::Undo, &["<Primary>z"][..]),
            (Action::Redo, &["<Primary><Shift>z", "<Primary>y"][..]),
            (Action::Preferences, &["<Primary>comma"][..]),
            (Action::Shortcuts, &["<Primary>question"][..]),
        ] {
            app.set_accels_for_action(&format!("win.{}", action.id()), accelerators);
        }
    }

    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    let menu_button = gtk::MenuButton::builder()
        .icon_name(project_menu::ICON)
        .tooltip_text(tr!(project_menu::LABEL).as_ref())
        .has_frame(false)
        .popover(&popover)
        .build();
    shrimply_components_gtk::ui::adaptive_menu(&menu_button, "win", window);
    let export_button =
        export::build_export_button(window, session.project.clone(), session.preferences.clone());
    let header_right = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    header_right.append(&export_button);
    header_right.append(&menu_button);
    header_bar.pack_end(&header_right);
}

type HostAction = fn(&adw::ApplicationWindow, &adw::ToastOverlay, &Rc<EditorSession>);

fn host_action(action: Action) -> Option<HostAction> {
    Some(match action {
        Action::NewProject => |window, _, _| {
            if let Err(error) = launch_sibling("shrimply", None) {
                show_error_dialog(window, "Could not create project", &error);
            }
        },
        Action::OpenProject => |window, _, _| show_open_project_dialog(window),
        Action::SaveAs => show_save_as_dialog,
        Action::Shortcuts => |window, _, _| show_shortcuts_dialog(window),
        Action::Preferences => |window, _, session| {
            shrimply_preferences_gtk::page::show_preferences_dialog(
                window,
                session.preferences.clone(),
            );
        },
        Action::About => |window, _, _| show_about_dialog(window),
        _ => return None,
    })
}

fn append_entries(menu: &gio::Menu, entries: &[Entry], register: &mut impl FnMut(Action)) {
    for entry in entries {
        match entry {
            Entry::Action(action) => {
                register(*action);
                menu.append_i18n(action.label(), &format!("win.{}", action.id()));
            }
            Entry::Submenu(label, entries) => {
                let submenu = gio::Menu::new();
                append_entries(&submenu, entries, register);
                menu.append_submenu_i18n(label, &submenu);
            }
        }
    }
}

fn add_action<F>(window: &adw::ApplicationWindow, name: &str, activate: F) -> gio::SimpleAction
where
    F: Fn() + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| activate());
    window.add_action(&action);
    action
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
    shrimply_components_gtk::project_open::open_project(&window.clone(), move |result| {
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
    let window = window.clone();
    let toasts = toasts.clone();
    let session = session.clone();
    gtk::glib::spawn_future_local(async move {
        let selected = shrimply_components_gtk::project_save::save_as(
            window.upcast_ref::<gtk::Window>(),
            shrimply_cross_ui_core::editor::suggested_save_as_path(),
        )
        .await;
        let (path, format) = match selected {
            Ok(Some(selected)) => selected,
            Ok(None) => return,
            Err(error) => {
                show_error_dialog(&window, "Could not save project", &error);
                return;
            }
        };
        let destination = format.normalize_path(path.clone());
        if destination != path && destination.exists() {
            let confirm = adw::AlertDialog::new(
                Some(tr!("Replace existing project?").as_ref()),
                Some(&destination.to_string_lossy()),
            );
            confirm.add_response("cancel", tr!("Cancel").as_ref());
            confirm.add_response("replace", tr!("Replace").as_ref());
            confirm.set_close_response("cancel");
            confirm.set_default_response(Some("cancel"));
            confirm.set_response_appearance("replace", adw::ResponseAppearance::Destructive);
            if confirm.choose_future(Some(&window)).await != "replace" {
                return;
            }
        }
        if let Err(error) = session.save_as(destination) {
            show_error_dialog(&window, "Could not save project", &error);
            return;
        }
        shrimply_components_gtk::toast::show_confirmation(
            &toasts,
            "Project saved to the new location",
        );
    });
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
    use shrimply_components_core::about;
    let dialog = adw::AboutDialog::builder()
        .application_name(about::NAME)
        .application_icon(about::ICON_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(about::DESCRIPTION)
        .developer_name(about::DEVELOPER)
        .license_type(gtk::License::Gpl30)
        .title(tr!("About Shrimply").as_ref())
        .build();
    for &(heading, entries) in about::CREDIT_SECTIONS {
        dialog.add_credit_section(Some(tr!(heading).as_ref()), entries);
    }
    for &link in about::LINKS {
        match link {
            about::WEBSITE => dialog.set_website(about::WEBSITE.1),
            about::REPORT_ISSUE => dialog.set_issue_url(about::REPORT_ISSUE.1),
            (label, url) => dialog.add_link(tr!(label).as_ref(), url),
        }
    }
    dialog.present(Some(window));
}
