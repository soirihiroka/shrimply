use adw::prelude::*;
use gtk::{gdk, gio};

pub(super) enum PresentedMenu {
    Popover(gtk::Popover),
    Sheet(adw::Dialog),
}

impl PresentedMenu {
    pub fn close(&self) {
        match self {
            Self::Popover(popover) => popover.popdown(),
            Self::Sheet(dialog) => {
                dialog.close();
            }
        }
    }

    pub fn close_on_click(&self, button: &gtk::Button) {
        match self {
            Self::Popover(popover) => {
                let popover = popover.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(popover) = popover.upgrade() {
                        popover.popdown();
                    }
                });
            }
            Self::Sheet(dialog) => {
                let dialog = dialog.downgrade();
                button.connect_clicked(move |_| {
                    if let Some(dialog) = dialog.upgrade() {
                        dialog.close();
                    }
                });
            }
        }
    }
}

pub(super) fn sheet(
    area: &gtk::GLArea,
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    custom_children: &[gtk::Widget],
) -> PresentedMenu {
    let custom_children = custom_children
        .iter()
        .enumerate()
        .map(|(index, child)| (format!("control-{index}"), child.clone()))
        .collect::<Vec<_>>();
    let dialog =
        shrimply_components_gtk::ui::action_menu_sheet(menu, "timeline", actions, &custom_children);
    dialog.present(Some(area));
    PresentedMenu::Sheet(dialog)
}

pub(super) fn bottom_sheet(area: &gtk::GLArea, content: &impl IsA<gtk::Widget>) -> adw::Dialog {
    let dialog = shrimply_components_gtk::ui::menu_sheet(content, None);
    dialog.present(Some(area));
    dialog
}

pub(super) fn popup(
    area: &gtk::GLArea,
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    custom_children: &[gtk::Widget],
    x: f64,
    y: f64,
) -> gtk::PopoverMenu {
    let parent = area.parent().expect("timeline GLArea must have a parent");
    let point = area
        .compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
        .expect("timeline coordinates must translate to its parent");
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    for (index, child) in custom_children.iter().enumerate() {
        assert!(popover.add_child(child, &format!("control-{index}")));
    }
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_halign(gtk::Align::Start);
    popover.insert_action_group("timeline", Some(actions));
    popover.set_parent(&parent);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(
        point.x() as i32,
        point.y() as i32,
        1,
        1,
    )));
    shrimply_components_gtk::ui::unparent_on_close(&popover);
    popover.popup();
    popover
}
