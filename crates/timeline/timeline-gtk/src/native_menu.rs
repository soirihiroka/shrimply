use gtk::gio;
use gtk::glib::variant::ToVariant as _;
use gtk::prelude::MenuModelExt as _;
use shrimply_timeline_core::{ContextMenu, ContextMenuEntry};

pub struct MenuModel {
    pub menu: gio::Menu,
}

pub fn menu_model(contract: &ContextMenu) -> MenuModel {
    let menu = gio::Menu::new();
    for entries in &contract.sections {
        let section = gio::Menu::new();
        for entry in entries {
            match entry {
                ContextMenuEntry::Action(item) => {
                    let action = format!("timeline.{}", item.action.id());
                    let menu_item =
                        shrimply_gtk_components::ui::menu_item_i18n(item.label(), &action);
                    let icon = match item.action {
                        shrimply_timeline_core::ContextMenuAction::Copy => {
                            Some("edit-copy-symbolic")
                        }
                        shrimply_timeline_core::ContextMenuAction::Paste => {
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
                    menu_item.set_attribute_value("custom", Some(&"speed-control".to_variant()));
                    section.append_item(&menu_item);
                }
            }
        }
        if section.n_items() > 0 {
            menu.append_section(None, &section);
        }
    }
    MenuModel { menu }
}
