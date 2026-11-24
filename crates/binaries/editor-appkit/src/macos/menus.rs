use super::{Editor, layout};
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSMenuToolbarItem,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, ns_string};
use shrimply_cross_ui_core::editor::project_menu::{self, Action, Entry};

fn item(
    menu: &NSMenu,
    title: &str,
    key: &str,
    action: Option<Sel>,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    };
    item.setEnabled(action.is_some());
    menu.addItem(&item);
    item
}

fn submenu(parent: &NSMenu, title: &str, mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    menu.setAutoenablesItems(false);
    let item = item(parent, title, "", None, mtm);
    item.setSubmenu(Some(&menu));
    item.setEnabled(true);
    menu
}

pub fn export_menu(editor: &Editor) -> Retained<NSMenu> {
    let mtm = editor.mtm();
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Export"));
    menu.setAutoenablesItems(false);
    let video = item(&menu, "Export video", "e", Some(sel!(exportVideo:)), mtm);
    unsafe {
        video.setTarget(Some(editor));
    }
    let captions = item(
        &menu,
        "Export Captions…",
        "",
        Some(sel!(exportCaptions:)),
        mtm,
    );
    unsafe {
        captions.setTarget(Some(editor));
    }
    menu
}

pub fn install(editor: &Editor) {
    let mtm = editor.mtm();
    let app = NSApplication::sharedApplication(mtm);
    let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
    let application = submenu(&main, "Shrimply", mtm);
    command(&application, editor, Action::About, "");
    application.addItem(&NSMenuItem::separatorItem(mtm));
    command(&application, editor, Action::Preferences, ",");
    application.addItem(&NSMenuItem::separatorItem(mtm));
    item(
        &application,
        "Quit Shrimply",
        "q",
        Some(sel!(terminate:)),
        mtm,
    );

    let file = submenu(&main, "File", mtm);
    command(&file, editor, Action::Import, "i");
    command(&file, editor, Action::Save, "s");
    let save_as = command(&file, editor, Action::SaveAs, "s");
    save_as
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    file.addItem(&NSMenuItem::separatorItem(mtm));
    let export = item(&file, "Export", "", None, mtm);
    export.setSubmenu(Some(&export_menu(editor)));
    export.setEnabled(true);
    file.addItem(&NSMenuItem::separatorItem(mtm));
    item(&file, "Close Window", "w", Some(sel!(performClose:)), mtm);

    let edit = submenu(&main, "Edit", mtm);
    edit.setAutoenablesItems(true);
    command(&edit, editor, Action::Undo, "z");
    command(&edit, editor, Action::Redo, "z")
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    edit.addItem(&NSMenuItem::separatorItem(mtm));
    item(&edit, "Cut", "x", Some(sel!(cut:)), mtm);
    item(&edit, "Copy", "c", Some(sel!(copy:)), mtm);
    item(&edit, "Paste", "v", Some(sel!(paste:)), mtm);
    item(&edit, "Select All", "a", Some(sel!(selectAll:)), mtm);

    let view = submenu(&main, "View", mtm);
    let mut view_items = Vec::new();
    for (title, key, action) in [
        ("Inspector", "1", sel!(toggleInspector:)),
        ("Timeline", "2", sel!(toggleTimeline:)),
        ("Fullscreen Preview", "f", sel!(togglePreviewFullscreen:)),
    ] {
        let item = item(&view, title, key, Some(action), mtm);
        unsafe { item.setTarget(Some(editor)) };
        if key == "f" {
            item.setKeyEquivalentModifierMask(
                NSEventModifierFlags::Command | NSEventModifierFlags::Control,
            );
        }
        view_items.push(item);
    }
    editor
        .ivars()
        .view_items
        .set(view_items)
        .expect("view menu already installed");
    let help = submenu(&main, "Help", mtm);
    command(&help, editor, Action::Shortcuts, "");
    app.setHelpMenu(Some(&help));
    app.setMainMenu(Some(&main));
}

fn project_options(editor: &Editor) -> Retained<NSMenu> {
    let mtm = editor.mtm();
    let label = shrimply_i18n::text(project_menu::LABEL);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(&label));
    for (index, section) in
        project_menu::sections(|action| Editor::project_host_action(action).is_some())
            .into_iter()
            .enumerate()
    {
        if index > 0 {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        append_entries(editor, &menu, &section);
    }
    menu
}

fn command(menu: &NSMenu, editor: &Editor, action: Action, key: &str) -> Retained<NSMenuItem> {
    let title = shrimply_i18n::text(action.label());
    let row = item(
        menu,
        &title,
        key,
        Some(sel!(activateProjectMenu:)),
        editor.mtm(),
    );
    row.setTag(action as isize);
    unsafe { row.setTarget(Some(editor)) };
    row
}

fn append_entries(editor: &Editor, menu: &NSMenu, entries: &[Entry]) {
    let mtm = editor.mtm();
    for entry in entries {
        match entry {
            Entry::Action(action) => {
                command(menu, editor, *action, "");
            }
            Entry::Submenu(label, children) => {
                let title = shrimply_i18n::text(label);
                append_entries(editor, &submenu(menu, &title, mtm), children)
            }
        }
    }
}

pub(super) fn selected_action(item: &NSMenuItem) -> Action {
    let index = usize::try_from(item.tag()).expect("project menu action index");
    Action::ALL[index]
}

pub fn toolbar_identifiers() -> Retained<NSArray<NSString>> {
    NSArray::from_slice(&[
        ns_string!("inspector"),
        ns_string!("timeline"),
        unsafe { NSToolbarFlexibleSpaceItemIdentifier },
        ns_string!("export"),
        ns_string!("project-options"),
    ])
}

pub fn toolbar_item(editor: &Editor, identifier: &NSString) -> Option<Retained<NSToolbarItem>> {
    let identifier_name = identifier.to_string();
    let menu = match identifier_name.as_str() {
        "export" => Some(("Export", "square.and.arrow.up", export_menu(editor))),
        "project-options" => Some((
            project_menu::LABEL,
            "ellipsis.circle",
            project_options(editor),
        )),
        _ => None,
    };
    if let Some((label, symbol, menu)) = menu {
        let item = NSMenuToolbarItem::initWithItemIdentifier(
            NSMenuToolbarItem::alloc(editor.mtm()),
            identifier,
        );
        let label = shrimply_i18n::text(label);
        let title = NSString::from_str(&label);
        item.setLabel(&title);
        item.setToolTip(Some(&title));
        item.setImage(Some(&layout::symbol(symbol, &label)));
        item.setMenu(&menu);
        item.setShowsIndicator(true);
        item.setBordered(true);
        return Some(item.into_super());
    }
    let (label, symbol, action) = match identifier_name.as_str() {
        "inspector" => ("Toggle Inspector", "sidebar.left", sel!(toggleInspector:)),
        "timeline" => (
            "Toggle Timeline",
            "rectangle.bottomthird.inset.filled",
            sel!(toggleTimeline:),
        ),
        _ => return None,
    };
    let item =
        NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(editor.mtm()), identifier);
    item.setNavigational(true);
    item.setLabel(&NSString::from_str(label));
    item.setToolTip(Some(&NSString::from_str(label)));
    item.setImage(Some(&layout::symbol(symbol, label)));
    unsafe {
        item.setTarget(Some(editor));
        item.setAction(Some(action));
    }
    Some(item)
}
