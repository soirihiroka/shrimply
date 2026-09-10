use super::{Editor, layout};
use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{DefinedClass, MainThreadOnly, sel};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSMenuToolbarItem,
    NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, ns_string};

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
    let about = item(
        &application,
        "About Shrimply",
        "",
        Some(sel!(showAbout:)),
        mtm,
    );
    unsafe { about.setTarget(Some(editor)) };
    application.addItem(&NSMenuItem::separatorItem(mtm));
    let settings = item(
        &application,
        "Settings…",
        ",",
        Some(sel!(showSettings:)),
        mtm,
    );
    unsafe { settings.setTarget(Some(editor)) };
    application.addItem(&NSMenuItem::separatorItem(mtm));
    item(
        &application,
        "Quit Shrimply",
        "q",
        Some(sel!(terminate:)),
        mtm,
    );

    let file = submenu(&main, "File", mtm);
    let import = item(&file, "Import Media…", "i", Some(sel!(importMedia:)), mtm);
    unsafe {
        import.setTarget(Some(editor));
    }
    let save = item(&file, "Save", "s", Some(sel!(saveProject:)), mtm);
    unsafe {
        save.setTarget(Some(editor));
    }
    let save_as = item(&file, "Save As…", "s", Some(sel!(saveProjectAs:)), mtm);
    unsafe { save_as.setTarget(Some(editor)) };
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
    let undo = item(&edit, "Undo", "z", Some(sel!(undo:)), mtm);
    let redo = item(&edit, "Redo", "z", Some(sel!(redo:)), mtm);
    redo.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    unsafe {
        undo.setTarget(Some(editor));
        redo.setTarget(Some(editor));
    }
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
    let shortcuts = item(
        &help,
        "Keyboard Shortcuts",
        "",
        Some(sel!(showShortcuts:)),
        mtm,
    );
    unsafe { shortcuts.setTarget(Some(editor)) };
    app.setHelpMenu(Some(&help));
    app.setMainMenu(Some(&main));
}

pub fn toolbar_identifiers() -> Retained<NSArray<NSString>> {
    NSArray::from_slice(&[
        ns_string!("inspector"),
        ns_string!("timeline"),
        unsafe { NSToolbarFlexibleSpaceItemIdentifier },
        ns_string!("export"),
    ])
}

pub fn toolbar_item(editor: &Editor, identifier: &NSString) -> Option<Retained<NSToolbarItem>> {
    if identifier.to_string() == "export" {
        let item = NSMenuToolbarItem::initWithItemIdentifier(
            NSMenuToolbarItem::alloc(editor.mtm()),
            identifier,
        );
        item.setLabel(ns_string!("Export"));
        item.setToolTip(Some(ns_string!("Export")));
        item.setImage(Some(&layout::symbol("square.and.arrow.up", "Export")));
        item.setMenu(&export_menu(editor));
        item.setShowsIndicator(true);
        item.setBordered(true);
        return Some(item.into_super());
    }
    let (label, symbol, action) = match identifier.to_string().as_str() {
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
