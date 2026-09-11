use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use shrimply_components_gtk::ui::I18nAlertDialogExt;

use super::TimelineRuntime;
use super::interaction::show_error_dialog;

pub(super) enum Content {
    Text(String),
    Files(Vec<PathBuf>),
    Texture(gdk::Texture),
    Url(String),
}

impl Content {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Files(_) => "file",
            Self::Texture(_) => "texture",
            Self::Url(_) => "URL",
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Origin {
    Clipboard,
    Drop,
}

#[derive(Clone, Copy)]
pub(super) enum Placement {
    Playhead,
    Timeline { x: f64, y: f64 },
}

pub(super) fn insert(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    content: Content,
    origin: Origin,
    placement: Placement,
) -> bool {
    let point = match placement {
        Placement::Playhead => None,
        Placement::Timeline { x, y } => Some(super::vec2(x as f32, y as f32)),
    };
    let content = match content {
        Content::Text(text) => {
            Ok(shrimply_timeline_skia::external_content::ExternalDrop::Text(text))
        }
        Content::Texture(texture) => {
            let bytes = texture.save_to_png_bytes();
            shrimply_timeline_skia::external_content::store_clipboard_image(bytes.as_ref()).map(
                |path| shrimply_timeline_skia::external_content::ExternalDrop::Files(vec![path]),
            )
        }
        Content::Files(paths) => stage_clipboard_paths(paths, origin)
            .map(shrimply_timeline_skia::external_content::ExternalDrop::Files),
        Content::Url(url) => {
            Ok(shrimply_timeline_skia::external_content::ExternalDrop::ImageUrl(url))
        }
    };
    let result = content.and_then(|content| {
        runtime
            .borrow_mut()
            .scene
            .perform_external_drop(content, point)
    });
    match result {
        Ok(shrimply_timeline_skia::external_content::ExternalDropAction::Complete) => {
            area.queue_render();
            true
        }
        Ok(shrimply_timeline_skia::external_content::ExternalDropAction::Importing(_)) => {
            area.queue_render();
            true
        }
        Ok(shrimply_timeline_skia::external_content::ExternalDropAction::ConfirmRemux {
            paths,
            batch,
        }) => {
            confirm_remux(area, runtime, paths, point, batch);
            true
        }
        Err(error) => {
            show_error_dialog(area, "Could not insert timeline content", &error);
            false
        }
    }
}

fn confirm_remux(
    area: &gtk::GLArea,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    paths: Vec<PathBuf>,
    point: Option<glam::Vec2>,
    batch: shrimply_timeline_skia::import_queue::BatchId,
) {
    let dialog = adw::AlertDialog::new(
        Some("Remux MKV/WebM to MP4?"),
        Some("MP4 is the supported timeline format. The source files will be kept."),
    );
    dialog.add_responses_i18n(&[("cancel", "Cancel"), ("remux", "Remux")]);
    dialog.set_close_response("cancel");
    dialog.set_default_response(Some("remux"));
    dialog.set_response_appearance("remux", adw::ResponseAppearance::Suggested);
    let area = area.clone();
    let runtime = runtime.clone();
    dialog.choose(
        Some(area.clone().upcast_ref::<gtk::Widget>()),
        None::<&gio::Cancellable>,
        move |response| {
            if response.as_str() == "remux"
                && let Err(error) = runtime
                    .borrow_mut()
                    .scene
                    .begin_external_remux(paths, point, batch)
            {
                show_error_dialog(&area, "Could not remux source file", &error);
            }
            area.queue_render();
        },
    );
}

fn stage_clipboard_paths(paths: Vec<PathBuf>, origin: Origin) -> Result<Vec<PathBuf>, String> {
    if matches!(origin, Origin::Drop) {
        return Ok(paths);
    }
    paths
        .into_iter()
        .map(|path| {
            shrimply_timeline_skia::external_content::store_clipboard_visual_file(&path)
                .map(|stored| stored.unwrap_or(path))
        })
        .collect()
}

pub(super) fn from_value(value: &glib::Value) -> Option<Content> {
    value
        .get::<gdk::FileList>()
        .ok()
        .and_then(|files| content_from_files(files.files()))
        .or_else(|| {
            value
                .get::<gio::File>()
                .ok()
                .and_then(|file| content_from_files([file]))
        })
        .or_else(|| value.get::<gdk::Texture>().ok().map(Content::Texture))
        .or_else(|| {
            value
                .get::<glib::Bytes>()
                .ok()
                .and_then(|bytes| std::str::from_utf8(bytes.as_ref()).ok().map(str::to_owned))
                .map(content_from_text)
        })
        .or_else(|| value.get::<String>().ok().map(content_from_text))
}

fn content_from_files(files: impl IntoIterator<Item = gio::File>) -> Option<Content> {
    let mut paths = Vec::new();
    let mut url = None;
    for file in files {
        if let Some(path) = file_path(&file) {
            paths.push(path);
        } else {
            let uri = file.uri();
            if url.is_none() && (uri.starts_with("https://") || uri.starts_with("http://")) {
                url = Some(uri.into());
            }
        }
    }
    if paths.is_empty() {
        url.map(Content::Url)
    } else {
        Some(Content::Files(paths))
    }
}

pub(super) fn content_from_text(text: String) -> Content {
    let paths = uri_paths(&text);
    if !paths.is_empty() {
        return Content::Files(paths);
    }
    match shrimply_timeline_skia::external_content::classify_external_text(text) {
        shrimply_timeline_skia::external_content::ExternalText::Text(text) => Content::Text(text),
        shrimply_timeline_skia::external_content::ExternalText::ImageUrl(url) => Content::Url(url),
    }
}

pub(super) fn supported_uri_paths(text: &str) -> Vec<PathBuf> {
    uri_paths(text)
        .into_iter()
        .filter(|path| shrimply_timeline_skia::import::file_kind(path).is_some())
        .collect()
}

fn file_path(file: &gio::File) -> Option<PathBuf> {
    file.path().or_else(|| {
        let uri = file.uri();
        glib::filename_from_uri(uri.as_str())
            .ok()
            .map(|(path, _)| path)
    })
}

fn uri_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line == "copy" || line == "cut" {
                return None;
            }
            glib::filename_from_uri(line).ok().map(|(path, _)| path)
        })
        .collect()
}
