use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use super::TimelineRuntime;
use super::external_content::{self, Content, Origin, Placement};

const FILE_MIME_TYPES: &[&str] = &["x-special/gnome-copied-files", "text/uri-list"];
pub(super) const TIMELINE_MARKER: &str = shrimply_timeline_skia::TIMELINE_CLIPBOARD_MARKER;

pub(super) fn paste(area: &gtk::GLArea, runtime: &Rc<RefCell<TimelineRuntime>>) {
    let clipboard = area.display().clipboard();
    let formats = clipboard.formats();
    if formats.contains_type(gdk::FileList::static_type())
        || FILE_MIME_TYPES
            .iter()
            .any(|mime| formats.contain_mime_type(mime))
    {
        paste_file(clipboard, area, runtime);
    } else if formats.contains_type(gdk::Texture::static_type())
        || formats
            .mime_types()
            .iter()
            .any(|mime| mime.starts_with("image/"))
    {
        let area = area.clone();
        let runtime = runtime.clone();
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |result| {
            let Some(texture) = result.ok().flatten() else {
                return;
            };
            external_content::insert(
                &area,
                &runtime,
                Content::Texture(texture),
                Origin::Clipboard,
                Placement::Playhead,
            );
        });
    } else {
        let area = area.clone();
        let runtime = runtime.clone();
        let paste = runtime.borrow().scene.clipboard_paste();
        clipboard.read_text_async(None::<&gio::Cancellable>, move |result| {
            let Some(text) = result.ok().flatten() else {
                return;
            };
            if text == TIMELINE_MARKER {
                let result = runtime.borrow_mut().scene.paste_clipboard(paste);
                if let Err(error) = result {
                    super::interaction::show_error_dialog(
                        &area,
                        "Could not paste timeline items",
                        &error,
                    );
                }
                area.queue_render();
            } else {
                external_content::insert(
                    &area,
                    &runtime,
                    external_content::content_from_text(text.into()),
                    Origin::Clipboard,
                    Placement::Playhead,
                );
            }
        });
    }
}

fn paste_file(
    clipboard: gdk::Clipboard,
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let area = area.clone();
    let runtime = runtime.clone();
    if clipboard
        .formats()
        .contains_type(gdk::FileList::static_type())
    {
        clipboard.read_value_async(
            gdk::FileList::static_type(),
            glib::Priority::DEFAULT,
            None::<&gio::Cancellable>,
            move |result| {
                let Some(content) = result
                    .ok()
                    .and_then(|value| external_content::from_value(&value))
                else {
                    return;
                };
                external_content::insert(
                    &area,
                    &runtime,
                    content,
                    Origin::Clipboard,
                    Placement::Playhead,
                );
            },
        );
        return;
    }

    clipboard.read_async(
        FILE_MIME_TYPES,
        glib::Priority::DEFAULT,
        None::<&gio::Cancellable>,
        move |result| {
            let Ok((stream, _)) = result else {
                return;
            };
            gio::prelude::InputStreamExt::read_bytes_async(
                &stream,
                1_048_576,
                glib::Priority::DEFAULT,
                None::<&gio::Cancellable>,
                move |result| {
                    let paths = result
                        .ok()
                        .and_then(|bytes| {
                            std::str::from_utf8(bytes.as_ref()).ok().map(str::to_owned)
                        })
                        .map(|text| external_content::supported_uri_paths(&text))
                        .unwrap_or_default();
                    if paths.is_empty() {
                        return;
                    }
                    external_content::insert(
                        &area,
                        &runtime,
                        Content::Files(paths),
                        Origin::Clipboard,
                        Placement::Playhead,
                    );
                },
            );
        },
    );
}
