use super::*;
use shrimply_components_gtk::tr;

pub(crate) fn open_track_import_dialog(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    targets: Vec<TrackKey>,
    position: Time,
) {
    let label = "Import to Track";
    let dialog = gtk::FileDialog::builder()
        .title(tr!(label).as_ref())
        .build();
    let area = area.downgrade();
    let runtime = Rc::downgrade(runtime);
    shrimply_components_gtk::file_picker::open(
        label,
        &dialog,
        None::<&gtk::Window>,
        move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            let (Some(area), Some(runtime)) = (area.upgrade(), runtime.upgrade()) else {
                return;
            };
            let result = runtime
                .borrow_mut()
                .scene
                .import_track_file_at(path, &targets, position);
            match result {
                Ok(started) if started.needs_remux => {
                    crate::external_content::confirm_remux(&area, &runtime, started.batch);
                }
                Ok(_) => {}
                Err(error) => show_error_dialog(&area, "Could not import file", &error),
            }
            area.queue_render();
        },
    );
}

pub(crate) fn show_error_dialog(area: &gtk::GLArea, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    dialog.choose(
        Some(area.upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        |_| {},
    );
}
