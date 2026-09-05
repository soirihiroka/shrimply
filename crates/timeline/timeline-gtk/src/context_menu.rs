use adw::prelude::*;
use gtk::{gdk, gio};

pub(super) fn popup(
    area: &gtk::GLArea,
    menu: &gio::Menu,
    actions: &gio::SimpleActionGroup,
    custom_child: Option<&gtk::Widget>,
    x: f64,
    y: f64,
) -> gtk::PopoverMenu {
    let parent = area.parent().expect("timeline GLArea must have a parent");
    let point = area
        .compute_point(&parent, &gtk::graphene::Point::new(x as f32, y as f32))
        .expect("timeline coordinates must translate to its parent");
    let popover = gtk::PopoverMenu::from_model(Some(menu));
    if let Some(child) = custom_child {
        assert!(popover.add_child(child, "speed-control"));
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
    popover.popup();
    popover
}
