use adw::prelude::*;

pub fn show_export_finished_for_widget(
    widget: &impl IsA<gtk::Widget>,
    title: &str,
    path: &std::path::Path,
) {
    if let Some(parent) = widget.root().and_downcast::<adw::ApplicationWindow>() {
        show_export_finished(&parent, &crate::i18n::text(title), path);
    }
}

pub fn show_export_finished(parent: &adw::ApplicationWindow, title: &str, path: &std::path::Path) {
    let dialog = adw::AlertDialog::new(Some(title), None);
    dialog.add_response("close", &crate::i18n::text("Close"));
    dialog.add_response("files", &crate::i18n::text("Show in Files"));
    dialog.set_close_response("close");
    dialog.set_default_response(Some("close"));
    let path = path.to_path_buf();
    let window = parent.clone();
    dialog.choose(
        Some(parent),
        None::<&gtk::gio::Cancellable>,
        move |answer| {
            if answer.as_str() == "files" {
                crate::desktop_open::reveal_file(window.upcast_ref(), path);
            }
        },
    );
}
