use std::cell::RefCell;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::player_state::{self, ProjectChange, SharedPlayerState};
use crate::project::{CaptionItem, Project, Time, VideoItem, VideoItemContent, VideoTrack};
use crate::selection_state::SharedSelectionState;

use super::interaction::{
    ask_remux_then_import_at, content_y, import_path_at, set_timeline_selection, show_error_dialog,
};
use super::items::{self, ItemKey, TrackKind};
use super::{TimelineRuntime, import, timeline_x, x_to_time};

const CLIPBOARD_MEDIA_DIR: &str = "media/clipboard";

pub(super) enum Content {
    Text(String),
    File(PathBuf),
    Texture(gdk::Texture),
    Url(String),
}

impl Content {
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::File(_) => "file",
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

#[derive(Clone)]
pub(super) struct TextPreview {
    pub(super) text: String,
    pub(super) kind: TrackKind,
    pub(super) track_index: usize,
    pub(super) start: Time,
    pub(super) end: Time,
}

pub(super) fn text_preview(
    project: &Project,
    runtime: &TimelineRuntime,
    text: String,
    x: f64,
    y: f64,
) -> Option<TextPreview> {
    if text.is_empty() || x < timeline_x() {
        return None;
    }
    let (kind, track_index, _) = items::track_at_y(project, y + runtime.view.scroll_y)?;
    let start = Time::from_seconds_f64(x_to_time(
        x,
        runtime.view.scroll_seconds,
        runtime.view.seconds_per_pixel,
    ));
    let start = runtime.snap_repository.snap(start).unwrap_or(start);
    let mut end = start
        .saturating_add(runtime.default_visual_duration)
        .snapped(project.frame_step());
    match kind {
        TrackKind::Caption => {
            for item in &project.caption_tracks.get(track_index)?.items {
                if item.start <= start && start < item.end {
                    return None;
                }
                if item.start > start {
                    end = end.min(item.start);
                    break;
                }
            }
        }
        TrackKind::Video => {
            for item in &project.video_tracks.get(track_index)?.items {
                if item.start <= start && start < item.end {
                    return None;
                }
                if item.start > start {
                    end = end.min(item.start);
                    break;
                }
            }
        }
        TrackKind::Audio => return None,
    }
    (end > start).then_some(TextPreview {
        text,
        kind,
        track_index,
        start,
        end,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn insert(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    content: Content,
    origin: Origin,
    placement: Placement,
) -> bool {
    match content {
        Content::Text(text) => insert_text(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            text,
            placement,
        ),
        Content::Texture(texture) => {
            let directory = crate::project::project_directory().join(CLIPBOARD_MEDIA_DIR);
            let path = directory.join(format!("{}.png", uuid::Uuid::new_v4()));
            if let Err(error) = fs::create_dir_all(&directory)
                .map_err(|error| error.to_string())
                .and_then(|()| {
                    texture
                        .save_to_png(&path)
                        .map_err(|error| error.to_string())
                })
            {
                show_error_dialog(area, "Could not store image", &error);
                return false;
            }
            insert_file(
                area,
                project,
                player_state,
                selection_state,
                runtime,
                path,
                origin,
                placement,
            )
        }
        Content::File(path) => insert_file(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            path,
            origin,
            placement,
        ),
        Content::Url(url) => download_image(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            url,
            origin,
            placement,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn download_image(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    url: String,
    origin: Origin,
    placement: Placement,
) -> bool {
    let directory = crate::project::project_directory().join(CLIPBOARD_MEDIA_DIR);
    let (sender, receiver) = mpsc::channel();
    let logged_url = url.clone();
    thread::spawn(move || {
        let result = (|| {
            let client = reqwest::blocking::Client::builder()
                .user_agent(concat!("shrimply/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|error| error.to_string())?;
            let response = client
                .get(&url)
                .header(reqwest::header::ACCEPT, "image/*")
                .send()
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            let response_content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            let extension = match response_content_type {
                Some(content_type) => {
                    image_extension_for_content_type(content_type).ok_or_else(|| {
                        format!("the dropped URL returned unsupported content type {content_type}")
                    })?
                }
                None => image_extension_for_url(&url).ok_or_else(|| {
                    "the dropped URL had no image content type or supported extension".to_string()
                })?,
            };
            let mut bytes = Vec::new();
            response
                .take(100 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > 100 * 1024 * 1024 {
                return Err("the dropped image is larger than 100 MiB".to_string());
            }
            fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
            let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), extension));
            fs::write(&path, bytes).map_err(|error| error.to_string())?;
            Ok(path)
        })();
        let _ = sender.send(result);
    });

    let area = area.clone();
    let project = project.clone();
    let player_state = player_state.clone();
    let selection_state = selection_state.clone();
    let runtime = runtime.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        match receiver.try_recv() {
            Ok(Ok(path)) => {
                if !insert_file(
                    &area,
                    &project,
                    &player_state,
                    &selection_state,
                    &runtime,
                    path,
                    origin,
                    placement,
                ) {
                    tracing::warn!("Downloaded dropped image could not be inserted");
                }
                glib::ControlFlow::Break
            }
            Ok(Err(error)) => {
                tracing::warn!("Could not download dropped image url={logged_url}: {error}");
                show_error_dialog(&area, "Could not import dropped image", &error);
                glib::ControlFlow::Break
            }
            Err(TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                tracing::warn!("Dropped image download worker disconnected");
                glib::ControlFlow::Break
            }
        }
    });
    true
}

fn image_extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next()?.trim() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" => Some("bmp"),
        "image/tiff" => Some("tiff"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

fn image_extension_for_url(url: &str) -> Option<&'static str> {
    let extension = url
        .split(['?', '#'])
        .next()?
        .rsplit_once('.')?
        .1
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        "bmp" => Some("bmp"),
        "tif" | "tiff" => Some("tiff"),
        "svg" => Some("svg"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_file(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    mut path: PathBuf,
    origin: Origin,
    placement: Placement,
) -> bool {
    let Some(kind) = import::file_kind(&path) else {
        return false;
    };
    if matches!(origin, Origin::Clipboard)
        && matches!(
            kind,
            import::FileKind::Image
                | import::FileKind::Gif
                | import::FileKind::Svg
                | import::FileKind::Pdf
        )
    {
        let directory = crate::project::project_directory().join(CLIPBOARD_MEDIA_DIR);
        if !path.starts_with(&directory) {
            let mut stored_path = directory.join(uuid::Uuid::new_v4().to_string());
            stored_path.set_extension(path.extension().expect("visual file has an extension"));
            if let Err(error) = fs::create_dir_all(&directory)
                .and_then(|()| fs::copy(&path, &stored_path).map(|_| ()))
            {
                show_error_dialog(area, "Could not store image", &error.to_string());
                return false;
            }
            path = stored_path;
        }
    }

    let (start, target) = match placement {
        Placement::Playhead => (
            player_state::snapshot(player_state).position,
            items::NewItemTarget::Automatic,
        ),
        Placement::Timeline { x, y } => {
            let runtime = runtime.borrow();
            let start = Time::from_seconds_f64(x_to_time(
                x,
                runtime.view.scroll_seconds,
                runtime.view.seconds_per_pixel,
            ));
            (
                runtime.snap_repository.snap(start).unwrap_or(start),
                items::NewItemTarget::AtY(content_y(runtime.view, y)),
            )
        }
    };
    if import::direct_media_kind(kind) {
        import_path_at(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            path,
            start,
            target,
        );
    } else if matches!(kind, import::FileKind::Mkv | import::FileKind::WebM) {
        ask_remux_then_import_at(
            area,
            project,
            player_state,
            selection_state,
            runtime,
            path,
            start,
            target,
        );
    } else {
        return false;
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn insert_text(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    text: String,
    placement: Placement,
) -> bool {
    let inserted = insert_text_core(
        project,
        player_state,
        selection_state,
        runtime,
        text,
        placement,
    );
    if inserted {
        area.queue_render();
    }
    inserted
}

pub(crate) fn insert_text_at_playhead_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    text: String,
) -> bool {
    insert_text_core(
        project,
        player_state,
        selection_state,
        runtime,
        text,
        Placement::Playhead,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_text_core(
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &Rc<RefCell<TimelineRuntime>>,
    text: String,
    placement: Placement,
) -> bool {
    if text.is_empty() {
        return false;
    }
    let preview = match placement {
        Placement::Playhead => None,
        Placement::Timeline { x, y } => {
            let project_state = project.borrow();
            let runtime = runtime.borrow();
            let Some(preview) = text_preview(&project_state, &runtime, text.clone(), x, y) else {
                return false;
            };
            Some(preview)
        }
    };

    let frame_step = project.borrow().frame_step();
    let start = preview
        .as_ref()
        .map_or_else(
            || player_state::snapshot(player_state).position,
            |p| p.start,
        )
        .snapped(frame_step);
    let end = preview.as_ref().map_or_else(
        || {
            start
                .saturating_add(runtime.borrow().default_visual_duration)
                .snapped(frame_step)
        },
        |p| p.end,
    );
    let text = preview.as_ref().map_or(text, |p| p.text.clone());
    let default_text_font_family = runtime.borrow().default_text_font_family.clone();
    let mut project_state = project.borrow_mut();
    let (kind, track_index) = preview
        .as_ref()
        .map(|p| (p.kind, p.track_index))
        .unwrap_or_else(|| {
            let track_index = project_state
                .video_tracks
                .iter()
                .position(|track| {
                    track
                        .items
                        .iter()
                        .all(|item| item.end <= start || item.start >= end)
                })
                .unwrap_or_else(|| {
                    project_state.video_tracks.push(VideoTrack::default());
                    project_state.video_tracks.len() - 1
                });
            (TrackKind::Video, track_index)
        });
    let item_index = match kind {
        TrackKind::Caption => {
            let Some(track) = project_state.caption_tracks.get_mut(track_index) else {
                return false;
            };
            if end <= start {
                return false;
            }
            items::insert_sorted(&mut track.items, CaptionItem::new(start, end, text))
        }
        TrackKind::Video => {
            let canvas_size = project_state.canvas_size;
            let Some(track) = project_state.video_tracks.get_mut(track_index) else {
                return false;
            };
            if end <= start {
                return false;
            }
            let mut item = VideoItem::text_item(canvas_size, start, end);
            let VideoItemContent::Text(content) = &mut item.content else {
                unreachable!("text item constructor returned another item type");
            };
            content.text = shrimply_core::timeline_value::TimelineValue::new_const(text);
            content.font_families = vec![default_text_font_family];
            items::insert_sorted(&mut track.items, item)
        }
        TrackKind::Audio => return false,
    };
    let selected = ItemKey {
        kind,
        track_index,
        item_index,
    };
    let duration = project_state.duration();
    crate::project::commit_edit(&project_state, "insert-external-text");
    drop(project_state);

    let project_state = project.borrow();
    set_timeline_selection(
        &project_state,
        selection_state,
        vec![selected],
        Some(selected),
    );
    player_state::refresh_project(
        player_state,
        ProjectChange {
            duration: Some(duration),
            video: kind == TrackKind::Video,
            captions: kind == TrackKind::Caption,
            inspector: true,
            ..ProjectChange::default()
        },
    );
    true
}

pub(super) fn from_value(value: &glib::Value) -> Option<Content> {
    value
        .get::<gdk::FileList>()
        .ok()
        .and_then(|files| files.files().into_iter().find_map(file_content))
        .or_else(|| value.get::<gio::File>().ok().and_then(file_content))
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

fn content_from_text(text: String) -> Content {
    first_uri_path(&text)
        .map(Content::File)
        .or_else(|| first_http_url(&text).map(Content::Url))
        .unwrap_or(Content::Text(text))
}

pub(super) fn supported_uri_path(text: &str) -> Option<PathBuf> {
    first_uri_path(text).filter(|path| import::file_kind(path).is_some())
}

fn file_path(file: gio::File) -> Option<PathBuf> {
    file.path().or_else(|| {
        let uri = file.uri();
        glib::filename_from_uri(uri.as_str())
            .ok()
            .map(|(path, _)| path)
    })
}

fn file_content(file: gio::File) -> Option<Content> {
    let uri = file.uri();
    file_path(file).map(Content::File).or_else(|| {
        (uri.starts_with("https://") || uri.starts_with("http://"))
            .then(|| Content::Url(uri.into()))
    })
}

fn first_uri_path(text: &str) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line == "copy" || line == "cut" {
            return None;
        }
        let (path, _) = glib::filename_from_uri(line).ok()?;
        Some(path)
    })
}

fn first_http_url(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://") || line.starts_with("http://"))
        .map(str::to_owned)
}
