use super::*;

pub(super) fn watch_updates(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let area = area.downgrade();
    let runtime = Rc::downgrade(runtime);
    let deletion_dialog_open = Rc::new(std::cell::Cell::new(false));
    glib::timeout_add_local(WAVEFORM_POLL_INTERVAL, move || {
        let (Some(area), Some(runtime)) = (area.upgrade(), runtime.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        let (changed, error) = {
            let mut runtime = runtime.borrow_mut();
            let changed = runtime.scene.update_media();
            while runtime.scene.take_external_import_event().is_some() {}
            (changed, runtime.scene.take_error())
        };
        if changed {
            area.queue_render();
        }
        if let Some(error) = error {
            crate::interaction::show_error_dialog(&area, "Timeline operation failed", &error);
        }
        if !deletion_dialog_open.get() {
            let request = runtime.borrow_mut().scene.take_source_deletion();
            if let Some(request) = request {
                deletion_dialog_open.set(true);
                crate::external_content::confirm_source_deletion(
                    &area,
                    request,
                    deletion_dialog_open.clone(),
                );
            }
        }
        glib::ControlFlow::Continue
    });
}
