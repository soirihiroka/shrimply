use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use imagesize::ImageType;
use shrimply_editor_state::player_state;
use shrimply_project_document::project::project_directory;

use super::*;

const CLIPBOARD_MEDIA_DIR: &str = "media/clipboard";
const MAX_CLIPBOARD_IMAGE_BYTES: usize = 100 * 1024 * 1024;
const IMAGE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub enum ExternalText {
    Text(String),
    ImageUrl(String),
}

pub enum ExternalDrop {
    Files(Vec<PathBuf>),
    Text(String),
    ImageUrl(String),
    Mask(String),
}

pub enum ExternalDropAction {
    Complete,
    Importing(crate::import_queue::BatchId),
    ConfirmRemux {
        paths: Vec<PathBuf>,
        batch: crate::import_queue::BatchId,
    },
}

pub struct ExternalImportEvent {
    pub batch: crate::import_queue::BatchId,
    pub retained_paths: Option<Vec<PathBuf>>,
}

pub(crate) struct PendingDownload {
    receiver: mpsc::Receiver<Result<OwnedFile, String>>,
    placement: crate::import_queue::Placement,
    batch: crate::import_queue::BatchId,
}

pub(crate) struct PendingRemux {
    receiver: mpsc::Receiver<Result<RemuxedFiles, String>>,
    target: RemuxTarget,
    batch: crate::import_queue::BatchId,
}

pub(crate) enum RemuxTarget {
    Timeline(crate::import_queue::Placement),
    Tracks {
        tracks: Vec<project::TrackAddress>,
        start: Time,
    },
}

pub(crate) struct RemuxedFiles {
    paths: Vec<PathBuf>,
    generated: Vec<OwnedFile>,
}

pub(crate) struct OwnedFile {
    path: PathBuf,
    keep: bool,
}

fn remux_files(paths: Vec<PathBuf>) -> mpsc::Receiver<Result<RemuxedFiles, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut generated = Vec::new();
        let result = (|| {
            let mut outputs = Vec::with_capacity(paths.len());
            for path in paths {
                match crate::import::file_kind(&path) {
                    Some(crate::import::FileKind::Mkv | crate::import::FileKind::WebM) => {
                        let output = crate::import::remux_mkv_to_mp4(&path)?;
                        generated.push(OwnedFile::new(output.clone()));
                        outputs.push(output);
                    }
                    Some(_) => outputs.push(path),
                    None => {
                        return Err(format!("{} has an unsupported file type", path.display()));
                    }
                }
            }
            Ok(outputs)
        })();
        match result {
            Err(error) => {
                let _ = sender.send(Err(error));
            }
            Ok(paths) => {
                let _ = sender.send(Ok(RemuxedFiles { paths, generated }));
            }
        }
    });
    receiver
}

impl OwnedFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn keep(mut self) {
        self.keep = true;
    }
}

impl Drop for OwnedFile {
    fn drop(&mut self) {
        if !self.keep
            && let Err(error) = fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), %error, "Could not remove abandoned external import file");
        }
    }
}

pub fn classify_external_text(text: String) -> ExternalText {
    let url = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("https://") || line.starts_with("http://"))
        .map(str::to_owned);
    url.map_or(ExternalText::Text(text), ExternalText::ImageUrl)
}

pub fn external_files_need_remux(paths: &[PathBuf]) -> Result<bool, String> {
    if paths.is_empty() {
        return Err("drop contains no files".into());
    }
    let mut remux = false;
    for path in paths {
        match crate::import::file_kind(path) {
            None => return Err(format!("{} has an unsupported file type", path.display())),
            Some(crate::import::FileKind::Vtt) => {
                return Err("VTT files must be imported through a caption track".into());
            }
            Some(crate::import::FileKind::Mkv | crate::import::FileKind::WebM) => remux = true,
            Some(_) => {}
        }
    }
    Ok(remux)
}

fn download_image(url: &str) -> Result<OwnedFile, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("shrimply/", env!("CARGO_PKG_VERSION")))
        .timeout(IMAGE_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "image/*")
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let extension = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(image_extension_for_content_type)
        .or_else(|| image_extension_for_url(url))
        .ok_or("the dropped URL did not resolve to a supported image")?;
    let mut bytes = Vec::new();
    response
        .take((MAX_CLIPBOARD_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    validate_clipboard_image_length(bytes.len())?;
    store_clipboard_image_with_extension(&bytes, extension).map(OwnedFile::new)
}

pub fn store_clipboard_image(bytes: &[u8]) -> Result<PathBuf, String> {
    store_clipboard_raster(bytes, None)
}

fn store_clipboard_image_with_extension(
    bytes: &[u8],
    extension_hint: &str,
) -> Result<PathBuf, String> {
    if extension_hint.eq_ignore_ascii_case("svg") {
        validate_clipboard_image_length(bytes.len())?;
        let text = std::str::from_utf8(bytes)
            .map_err(|error| format!("clipboard SVG is not UTF-8: {error}"))?;
        if !text.contains("<svg") {
            return Err("clipboard SVG does not contain an SVG document".into());
        }
        return store_clipboard_bytes(bytes, "svg");
    }
    store_clipboard_raster(bytes, Some(extension_hint))
}

fn image_extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type.split(';').next()?.trim() {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/avif" => Some("avif"),
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
        "avif" => Some("avif"),
        "svg" => Some("svg"),
        _ => None,
    }
}

pub fn validate_clipboard_image_length(length: usize) -> Result<(), String> {
    if length == 0 {
        return Err("clipboard image is empty".into());
    }
    if length > MAX_CLIPBOARD_IMAGE_BYTES {
        return Err("clipboard image is larger than 100 MiB".into());
    }
    Ok(())
}

pub fn store_clipboard_visual_file(path: &Path) -> Result<Option<PathBuf>, String> {
    let Some(kind) = crate::import::file_kind(path) else {
        return Ok(None);
    };
    if !matches!(
        kind,
        crate::import::FileKind::Image
            | crate::import::FileKind::Gif
            | crate::import::FileKind::Svg
            | crate::import::FileKind::Pdf
    ) {
        return Ok(None);
    }
    let directory = project_directory().join(CLIPBOARD_MEDIA_DIR);
    if path.starts_with(&directory) {
        return Ok(Some(path.to_owned()));
    }
    let bytes = read_clipboard_file(path)?;
    if matches!(
        kind,
        crate::import::FileKind::Image | crate::import::FileKind::Gif
    ) {
        return store_clipboard_raster(
            &bytes,
            path.extension().and_then(|extension| extension.to_str()),
        )
        .map(Some);
    }

    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .ok_or("clipboard visual file has no extension")?;
    if bytes.is_empty() {
        return Err("clipboard visual file is empty".into());
    }
    store_clipboard_bytes(&bytes, extension).map(Some)
}

fn read_clipboard_file(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| {
            format!(
                "could not open clipboard visual {}: {error}",
                path.display()
            )
        })?
        .take((MAX_CLIPBOARD_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "could not read clipboard visual {}: {error}",
                path.display()
            )
        })?;
    validate_clipboard_image_length(bytes.len())?;
    Ok(bytes)
}

fn store_clipboard_raster(bytes: &[u8], extension_hint: Option<&str>) -> Result<PathBuf, String> {
    validate_clipboard_image_length(bytes.len())?;

    let extension = match imagesize::image_type(bytes)
        .map_err(|error| format!("clipboard image cannot be parsed: {error}"))?
    {
        ImageType::Png => "png",
        ImageType::Jpeg => "jpg",
        ImageType::Webp => "webp",
        ImageType::Gif => "gif",
        ImageType::Heif(_)
            if extension_hint.is_some_and(|extension| extension.eq_ignore_ascii_case("avif")) =>
        {
            "avif"
        }
        _ => return Err("clipboard image uses an unsupported format".into()),
    };
    let size = imagesize::blob_size(bytes)
        .map_err(|error| format!("clipboard image is invalid: {error}"))?;
    if size.width == 0 || size.height == 0 {
        return Err("clipboard image has no pixels".into());
    }

    store_clipboard_bytes(bytes, extension)
}

fn store_clipboard_bytes(bytes: &[u8], extension: &str) -> Result<PathBuf, String> {
    let directory = project_directory().join(CLIPBOARD_MEDIA_DIR);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create clipboard media directory: {error}"))?;
    let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), extension));
    fs::write(&path, bytes).map_err(|error| format!("could not store clipboard image: {error}"))?;
    Ok(path)
}

#[derive(Clone)]
pub struct TextPreview {
    pub text: String,
    pub kind: TrackKind,
    pub track_index: usize,
    pub start: Time,
    pub end: Time,
}

impl crate::scene::Scene {
    pub fn update_external_files_preview(&mut self, paths: &[PathBuf], point: Vec2) -> bool {
        if external_files_need_remux(paths).is_err() {
            self.clear_drop_preview();
            return false;
        }
        let Some(path) = paths.first() else {
            self.clear_drop_preview();
            return false;
        };
        self.update_drop_preview(path.clone(), point)
    }

    fn external_placement(&self, point: Option<Vec2>) -> crate::import_queue::Placement {
        match point {
            Some(point) => {
                let start = crate::math::time_at_x(self.view(), point.x.into());
                crate::import_queue::Placement {
                    start: self.snap_repository.snap(start).unwrap_or(start),
                    target: crate::items::NewItemTarget::AtY(
                        f64::from(point.y).max(crate::metrics::RULER_HEIGHT) + self.view().scroll_y,
                    ),
                    collision: self.drag_collision_mode,
                }
            }
            None => crate::import_queue::Placement {
                start: player_state::current_time(&self.player),
                target: crate::items::NewItemTarget::Automatic,
                collision: self.drag_collision_mode,
            },
        }
    }

    pub fn enqueue_external_files(
        &mut self,
        paths: Vec<PathBuf>,
        point: Option<Vec2>,
    ) -> Result<crate::import_queue::BatchId, String> {
        if point.is_some_and(|point| !self.external_drop_target(point)) {
            return Err("files cannot be inserted at this timeline position".into());
        }
        if external_files_need_remux(&paths)? {
            return Err("MKV and WebM must be remuxed before timeline import".into());
        }
        let placement = self.external_placement(point);
        self.external_imports.enqueue(
            paths,
            &self.project.borrow(),
            placement,
            self.default_visual_duration,
        )
    }

    pub fn perform_external_drop(
        &mut self,
        content: ExternalDrop,
        point: Option<Vec2>,
    ) -> Result<ExternalDropAction, String> {
        match content {
            ExternalDrop::Files(paths) => {
                if point.is_some_and(|point| !self.external_drop_target(point)) {
                    return Err("files cannot be inserted at this timeline position".into());
                }
                if external_files_need_remux(&paths)? {
                    let batch = self.external_imports.reserve_batch();
                    return Ok(ExternalDropAction::ConfirmRemux { paths, batch });
                }
                return self
                    .enqueue_external_files(paths, point)
                    .map(ExternalDropAction::Importing);
            }
            ExternalDrop::Text(text) => {
                if !self.insert_external_text(text, point) {
                    return Err("text cannot be inserted at this timeline position".into());
                }
            }
            ExternalDrop::ImageUrl(url) => {
                if point.is_some_and(|point| !self.external_drop_target(point)) {
                    return Err("image cannot be inserted at this timeline position".into());
                }
                return Ok(ExternalDropAction::Importing(
                    self.enqueue_external_image_url(url, point),
                ));
            }
            ExternalDrop::Mask(modifier_id) => {
                let modifier_id = modifier_id
                    .parse()
                    .map_err(|error| format!("invalid mask modifier ID: {error}"))?;
                if !self.assign_mask_source_at(
                    modifier_id,
                    point.ok_or("mask drops require a timeline position")?,
                )? {
                    return Err("mask source requires a compatible video item".into());
                }
            }
        }
        Ok(ExternalDropAction::Complete)
    }

    pub fn begin_external_remux(
        &mut self,
        paths: Vec<PathBuf>,
        point: Option<Vec2>,
        batch: crate::import_queue::BatchId,
    ) -> Result<(), String> {
        if point.is_some_and(|point| !self.external_drop_target(point)) {
            return Err("files cannot be inserted at this timeline position".into());
        }
        if !external_files_need_remux(&paths)? {
            return self.external_imports.enqueue_reserved(
                batch,
                paths,
                &self.project.borrow(),
                self.external_placement(point),
                self.default_visual_duration,
            );
        }
        let placement = self.external_placement(point);
        self.external_remuxes.push_back(PendingRemux {
            receiver: remux_files(paths),
            target: RemuxTarget::Timeline(placement),
            batch,
        });
        Ok(())
    }

    pub fn begin_track_remux(
        &mut self,
        path: PathBuf,
        tracks: Vec<project::TrackAddress>,
        start: Time,
    ) -> Result<(), String> {
        if !external_files_need_remux(std::slice::from_ref(&path))? {
            return Err("Track source does not require remuxing".into());
        }
        let batch = self.external_imports.reserve_batch();
        self.external_remuxes.push_back(PendingRemux {
            receiver: remux_files(vec![path]),
            target: RemuxTarget::Tracks { tracks, start },
            batch,
        });
        Ok(())
    }

    pub fn enqueue_external_image_url(
        &mut self,
        url: String,
        point: Option<Vec2>,
    ) -> crate::import_queue::BatchId {
        let placement = self.external_placement(point);
        let batch = self.external_imports.reserve_batch();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(download_image(&url));
        });
        self.external_downloads.push_back(PendingDownload {
            receiver,
            placement,
            batch,
        });
        batch
    }

    pub(crate) fn poll_external_content(&mut self) -> bool {
        let mut changed = false;
        if let Some(remux) = self.external_remuxes.front() {
            match remux.receiver.try_recv() {
                Ok(result) => {
                    let pending = self
                        .external_remuxes
                        .pop_front()
                        .expect("external remux exists");
                    match result.and_then(|remuxed| {
                        self.external_owned_files
                            .insert(pending.batch, remuxed.generated);
                        match pending.target {
                            RemuxTarget::Timeline(placement) => {
                                self.external_imports.enqueue_reserved(
                                    pending.batch,
                                    remuxed.paths,
                                    &self.project.borrow(),
                                    placement,
                                    self.default_visual_duration,
                                )
                            }
                            RemuxTarget::Tracks { tracks, start } => {
                                self.external_imports.enqueue_track_addresses_reserved(
                                    pending.batch,
                                    remuxed.paths,
                                    &self.project.borrow(),
                                    tracks,
                                    start,
                                    self.default_visual_duration,
                                )
                            }
                        }
                    }) {
                        Ok(()) => changed = true,
                        Err(error) => {
                            self.remove_external_owned_files(pending.batch);
                            self.external_import_events.push_back(ExternalImportEvent {
                                batch: pending.batch,
                                retained_paths: None,
                            });
                            self.pending_errors.push_back(error);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    let pending = self
                        .external_remuxes
                        .pop_front()
                        .expect("external remux exists");
                    self.external_import_events.push_back(ExternalImportEvent {
                        batch: pending.batch,
                        retained_paths: None,
                    });
                    self.pending_errors
                        .push_back("Media remux worker stopped unexpectedly".into());
                }
            }
        }
        if let Some(download) = self.external_downloads.front() {
            match download.receiver.try_recv() {
                Ok(result) => {
                    let pending = self
                        .external_downloads
                        .pop_front()
                        .expect("external download exists");
                    match result.and_then(|file| {
                        self.external_owned_files.insert(pending.batch, vec![file]);
                        let path = self.external_owned_files[&pending.batch][0].path.clone();
                        self.external_imports.enqueue_reserved(
                            pending.batch,
                            vec![path],
                            &self.project.borrow(),
                            pending.placement,
                            self.default_visual_duration,
                        )
                    }) {
                        Ok(()) => changed = true,
                        Err(error) => {
                            self.remove_external_owned_files(pending.batch);
                            self.external_import_events.push_back(ExternalImportEvent {
                                batch: pending.batch,
                                retained_paths: None,
                            });
                            self.pending_errors.push_back(error);
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    let pending = self
                        .external_downloads
                        .pop_front()
                        .expect("external download exists");
                    self.external_import_events.push_back(ExternalImportEvent {
                        batch: pending.batch,
                        retained_paths: None,
                    });
                    self.pending_errors
                        .push_back("Image download worker stopped unexpectedly".into());
                }
            }
        }
        loop {
            let completion = self.external_imports.poll(&mut self.project.borrow_mut());
            let Some(completion) = completion else {
                break;
            };
            match crate::import::finish_track_import(
                &self.player,
                &self.selection,
                completion.result,
            ) {
                Ok(()) => {
                    for file in self
                        .external_owned_files
                        .remove(&completion.batch)
                        .unwrap_or_default()
                    {
                        file.keep();
                    }
                    self.external_import_events.push_back(ExternalImportEvent {
                        batch: completion.batch,
                        retained_paths: Some(completion.paths),
                    });
                    changed = true;
                }
                Err(error) => {
                    self.external_owned_files.remove(&completion.batch);
                    self.external_import_events.push_back(ExternalImportEvent {
                        batch: completion.batch,
                        retained_paths: None,
                    });
                    self.pending_errors.push_back(error);
                }
            }
        }
        changed
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.pending_errors.pop_front()
    }

    pub fn take_external_import_event(&mut self) -> Option<ExternalImportEvent> {
        self.external_import_events.pop_front()
    }

    fn remove_external_owned_files(&mut self, batch: crate::import_queue::BatchId) {
        self.external_owned_files.remove(&batch);
    }

    pub fn mask_drop_target(&self, modifier_id: uuid::Uuid, point: Vec2) -> bool {
        if !self.drop_surface_target(point) {
            return false;
        }
        let project = self.project.borrow();
        let Some(source @ project::ItemAddress::Video { .. }) =
            crate::folded_sequence::hit_projected_item(
                &project,
                self.view(),
                f64::from(point.x),
                f64::from(point.y),
            )
            .map(|hit| hit.key)
        else {
            return false;
        };
        mask_owner(&project, &source, modifier_id).is_ok()
    }

    pub fn assign_mask_source_at(
        &mut self,
        modifier_id: uuid::Uuid,
        point: Vec2,
    ) -> Result<bool, String> {
        if !self.drop_surface_target(point) {
            return Ok(false);
        }
        let source = {
            let project = self.project.borrow();
            crate::folded_sequence::hit_projected_item(
                &project,
                self.view(),
                f64::from(point.x),
                f64::from(point.y),
            )
            .map(|hit| hit.key)
        };
        let Some(source @ project::ItemAddress::Video { .. }) = source else {
            return Ok(false);
        };
        let changed = set_mask_source(&mut self.project.borrow_mut(), &source, modifier_id)?;
        if changed {
            player_state::refresh_project(
                &self.player,
                player_state::ProjectChange {
                    video: true,
                    inspector: true,
                    ..Default::default()
                },
            );
        }
        Ok(true)
    }

    pub fn insert_external_text(&mut self, text: String, point: Option<Vec2>) -> bool {
        if text.is_empty() {
            return false;
        }
        let preview = match point {
            Some(point) => {
                let Some(preview) = self.text_preview(text.clone(), point) else {
                    return false;
                };
                Some(preview)
            }
            None => None,
        };
        let frame_step = self.project.borrow().frame_step();
        let start = preview
            .as_ref()
            .map_or_else(
                || player_state::snapshot(&self.player).position,
                |preview| preview.start,
            )
            .snapped(frame_step);
        let end = preview.as_ref().map_or_else(
            || {
                start
                    .saturating_add(self.default_visual_duration)
                    .snapped(frame_step)
            },
            |preview| preview.end,
        );
        let text = preview
            .as_ref()
            .map_or(text, |preview| preview.text.clone());
        let mut project = self.project.borrow_mut();
        let (kind, track_index) = preview
            .as_ref()
            .map(|preview| (preview.kind, preview.track_index))
            .unwrap_or_else(|| {
                let track_index = project
                    .video_tracks
                    .iter()
                    .position(|track| {
                        track
                            .items
                            .iter()
                            .all(|item| item.end <= start || item.start >= end)
                    })
                    .unwrap_or_else(|| {
                        project.video_tracks.push(project::VideoTrack::default());
                        project.video_tracks.len() - 1
                    });
                (TrackKind::Video, track_index)
            });
        if end <= start {
            return false;
        }
        let item_index = match kind {
            TrackKind::Caption => {
                let Some(track) = project.caption_tracks.get_mut(track_index) else {
                    return false;
                };
                crate::items::insert_sorted(
                    &mut track.items,
                    project::CaptionItem::new(start, end, text),
                )
            }
            TrackKind::Video => {
                let canvas_size = project.canvas_size;
                let Some(track) = project.video_tracks.get_mut(track_index) else {
                    return false;
                };
                let mut item = project::VideoItem::text_item(canvas_size, start, end);
                let project::VideoItemContent::Text(content) = &mut item.content else {
                    unreachable!("text item constructor returned another item type");
                };
                content.text =
                    shrimply_property_model::timeline_value::TimelineValue::new_const(text);
                content.font_families = vec![self.default_text_font_family.clone()];
                crate::items::insert_sorted(&mut track.items, item)
            }
            TrackKind::Audio => return false,
        };
        let selected = crate::items::ItemKey {
            kind,
            track_index,
            item_index,
        };
        let duration = project.duration();
        project::commit_edit(&project, "insert-external-text");
        drop(project);
        crate::scene::pointer::set_timeline_selection(
            &self.project.borrow(),
            &self.selection,
            vec![selected],
            Some(selected),
        );
        player_state::refresh_project(
            &self.player,
            player_state::ProjectChange {
                duration: Some(duration),
                video: kind == TrackKind::Video,
                captions: kind == TrackKind::Caption,
                inspector: true,
                ..Default::default()
            },
        );
        true
    }
}

fn set_mask_source(
    project: &mut Project,
    source: &project::ItemAddress,
    modifier_id: uuid::Uuid,
) -> Result<bool, String> {
    use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect};

    let project::ItemAddress::Video { .. } = source else {
        return Err("mask source is not a video item".into());
    };
    if project.video_item(source).is_none() {
        return Err("mask source item is no longer available".into());
    }
    let owner = mask_owner(project, source, modifier_id)?;
    let mask = project
        .video_item_mut(&owner)
        .and_then(|item| {
            item.modifiers
                .iter_mut()
                .find(|modifier| modifier.id == modifier_id)
        })
        .and_then(|modifier| match &mut modifier.effect {
            ModifierEffect::Raster(effect) => match &mut **effect {
                RasterModifierEffect::Mask(mask) => Some(mask),
                _ => None,
            },
            _ => None,
        })
        .ok_or("mask modifier is no longer available")?;
    if mask.item_id == Some(source.item_id()) {
        return Ok(false);
    }
    mask.item_id = Some(source.item_id());
    project::commit_edit(project, "edit-mask-source");
    Ok(true)
}

fn mask_owner(
    project: &Project,
    source: &project::ItemAddress,
    modifier_id: uuid::Uuid,
) -> Result<project::ItemAddress, String> {
    use shrimply_visual_modifiers::{ModifierEffect, RasterModifierEffect};

    let owner = project
        .video_tracks_for_path(source.sequence_path())
        .ok_or("mask source sequence is no longer available")?
        .iter()
        .find_map(|track| {
            track.items.iter().find_map(|item| {
                item.modifiers
                    .iter()
                    .any(|modifier| {
                        modifier.id == modifier_id
                            && matches!(
                                modifier.effect,
                                ModifierEffect::Raster(ref effect)
                                    if matches!(&**effect, RasterModifierEffect::Mask(_))
                            )
                    })
                    .then(|| project::ItemAddress::Video {
                        sequence_path: source.sequence_path().to_vec(),
                        track_id: track.id,
                        item_id: item.id,
                    })
            })
        })
        .ok_or("mask modifier is no longer available")?;
    if owner.item_id() == source.item_id() {
        return Err("a mask source cannot be its own item".into());
    }
    Ok(owner)
}
