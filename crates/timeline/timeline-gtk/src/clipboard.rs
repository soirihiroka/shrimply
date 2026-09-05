use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::player_state::SharedPlayerState;
use crate::project::Project;
use crate::selection_state::SharedSelectionState;

use super::TimelineRuntime;
use super::external_content::{self, Content, Origin, Placement};
use super::interaction::paste_timeline_clipboard;

const FILE_MIME_TYPES: &[&str] = &["x-special/gnome-copied-files", "text/uri-list"];
pub(super) const TIMELINE_MARKER: &str = shrimply_timeline_core::TIMELINE_CLIPBOARD_MARKER;

pub(super) fn paste(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let clipboard = area.display().clipboard();
    let formats = clipboard.formats();
    if formats.contains_type(gdk::FileList::static_type())
        || FILE_MIME_TYPES
            .iter()
            .any(|mime| formats.contain_mime_type(mime))
    {
        paste_file(
            clipboard,
            area,
            project,
            player_state,
            selection_state,
            runtime,
        );
    } else if formats.contains_type(gdk::Texture::static_type())
        || formats
            .mime_types()
            .iter()
            .any(|mime| mime.starts_with("image/"))
    {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |result| {
            let Some(texture) = result.ok().flatten() else {
                return;
            };
            external_content::insert(
                &area,
                &project,
                &player_state,
                &selection_state,
                &runtime,
                Content::Texture(texture),
                Origin::Clipboard,
                Placement::Playhead,
            );
        });
    } else {
        let area = area.clone();
        let project = project.clone();
        let player_state = player_state.clone();
        let selection_state = selection_state.clone();
        let runtime = runtime.clone();
        let timeline_clipboard = runtime.borrow().scene.clipboard.clone();
        let timeline_sequence_scope = crate::selection_state::active_scope(&selection_state);
        clipboard.read_text_async(None::<&gio::Cancellable>, move |result| {
            let Some(text) = result.ok().flatten() else {
                return;
            };
            if text == TIMELINE_MARKER {
                if let Some(clipboard) = timeline_clipboard {
                    paste_timeline_clipboard(
                        &area,
                        &project,
                        &player_state,
                        &selection_state,
                        &clipboard,
                        &timeline_sequence_scope,
                    );
                }
            } else {
                external_content::insert(
                    &area,
                    &project,
                    &player_state,
                    &selection_state,
                    &runtime,
                    Content::Text(text.into()),
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
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
) {
    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
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
                    &project,
                    &player_state,
                    &selection_state,
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
                    let Some(path) = result
                        .ok()
                        .and_then(|bytes| {
                            std::str::from_utf8(bytes.as_ref()).ok().map(str::to_owned)
                        })
                        .and_then(|text| external_content::supported_uri_path(&text))
                    else {
                        return;
                    };
                    external_content::insert(
                        &area,
                        &project,
                        &player_state,
                        &selection_state,
                        &runtime,
                        Content::File(path),
                        Origin::Clipboard,
                        Placement::Playhead,
                    );
                },
            );
        },
    );
}
