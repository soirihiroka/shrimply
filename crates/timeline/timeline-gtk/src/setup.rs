use super::*;

pub(super) fn watch_updates(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let area = area.downgrade();
    let runtime = Rc::downgrade(runtime);
    glib::timeout_add_local(WAVEFORM_POLL_INTERVAL, move || {
        let (Some(area), Some(runtime)) = (area.upgrade(), runtime.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if runtime.borrow_mut().scene.update_media() {
            area.queue_render();
        }
        glib::ControlFlow::Continue
    });
}

pub(super) fn timeline_tool_button(icon_name: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::new();
    button.set_child(Some(&gtk::Image::from_icon_name(icon_name)));
    button.set_size_request(SIDEBAR_ICON_SIZE, SIDEBAR_ICON_SIZE);
    button.add_css_class("flat");
    button
}
