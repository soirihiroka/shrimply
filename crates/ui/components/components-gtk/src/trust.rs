use adw::prelude::*;
use shrimply_trust_core::{Kind, Review};

pub fn confirm(
    parent: &impl IsA<gtk::Widget>,
    review: &Review,
    respond: impl FnOnce(Option<Kind>) + 'static,
) {
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Trust {} executable files?", review.files.len()))
        .body(shrimply_trust_core::EXPLANATION)
        .build();
    let details = gtk::Label::builder()
        .label(review.details())
        .selectable(true)
        .xalign(0.0)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .child(&details)
        .min_content_width(420)
        .max_content_height(240)
        .propagate_natural_height(true)
        .build();
    let expander = gtk::Expander::builder()
        .label("Show files and folders")
        .child(&scroll)
        .build();
    dialog.set_extra_child(Some(&expander));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("files", &format!("Trust {} files", review.files.len()));
    dialog.add_response(
        "folders",
        &format!("Trust {} folders", review.folders.len()),
    );
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.choose(
        Some(parent),
        None::<&gtk::gio::Cancellable>,
        move |answer| {
            respond(match answer.as_str() {
                "files" => Some(Kind::File),
                "folders" => Some(Kind::Folder),
                _ => None,
            });
        },
    );
}

pub fn handle_imports(parent: &impl IsA<gtk::Widget>) {
    let requests = shrimply_trust_core::interactive_requests();
    let parent = parent.downgrade();
    let busy = std::rc::Rc::new(std::cell::Cell::new(false));
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        let Some(parent) = parent.upgrade() else {
            return gtk::glib::ControlFlow::Break;
        };
        for error in shrimply_trust_core::poll_edits() {
            let dialog = adw::AlertDialog::new(Some("Could not change source"), Some(&error));
            dialog.add_response("close", "Close");
            dialog.present(Some(&parent));
        }
        if !busy.get()
            && let Ok(request) = requests.try_recv()
        {
            busy.set(true);
            let busy = busy.clone();
            confirm(&parent, &request.review, {
                let review = request.review.clone();
                move |kind| {
                    let result = kind
                        .ok_or_else(|| "Trust approval was canceled".to_string())
                        .and_then(|kind| review.approve(kind));
                    let _ = request.response.send(result);
                    busy.set(false);
                }
            });
        }
        gtk::glib::ControlFlow::Continue
    });
}
