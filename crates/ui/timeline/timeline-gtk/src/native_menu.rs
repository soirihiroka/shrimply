use gtk::gio;
use gtk::glib::variant::ToVariant as _;
use gtk::prelude::MenuModelExt as _;
use shrimply_timeline_skia::{ContextMenu, ContextMenuEntry};

pub fn menu_model(contract: &ContextMenu) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut control_index = 0;
    for entries in &contract.sections {
        let section = gio::Menu::new();
        for entry in entries {
            match entry {
                ContextMenuEntry::Action(item) => {
                    let action = format!("timeline.{}", item.key());
                    let menu_item =
                        shrimply_components_gtk::ui::menu_item_i18n(contract.label(*item), &action);
                    let icon = match item.action {
                        shrimply_timeline_skia::ContextMenuAction::Copy => {
                            Some("edit-copy-symbolic")
                        }
                        shrimply_timeline_skia::ContextMenuAction::Paste => {
                            Some("edit-paste-symbolic")
                        }
                        _ => None,
                    };
                    if let Some(icon) = icon {
                        menu_item.set_icon(&gio::ThemedIcon::new(icon));
                    }
                    section.append_item(&menu_item);
                }
                ContextMenuEntry::Control(_) => {
                    let menu_item = gio::MenuItem::new(None, None);
                    menu_item.set_attribute_value(
                        "custom",
                        Some(&format!("control-{control_index}").to_variant()),
                    );
                    control_index += 1;
                    section.append_item(&menu_item);
                }
            }
        }
        if section.n_items() > 0 {
            menu.append_section(None, &section);
        }
    }
    menu
}
