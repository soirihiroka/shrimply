use objc2::{ClassType, rc::Retained};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSBitmapImageFileType, NSBitmapImageRep, NSModalResponseOK,
    NSOpenPanel, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF,
};
use objc2_foundation::{MainThreadMarker, NSArray, NSDictionary, NSString, NSURL};
use shrimply_cross_ui_core::editor::EditorSession;
use shrimply_editor_state::preferences;
use shrimply_timeline_skia::{
    DragCollisionMode, TrackKey, import,
    import_queue::{ImportQueue, ImportStart, Placement, SourceDeletion},
    items::NewItemTarget,
};
use std::path::PathBuf;

pub(super) struct ScopedUrl {
    url: Retained<NSURL>,
    scoped: bool,
}

impl ScopedUrl {
    pub(super) fn new(url: Retained<NSURL>) -> Self {
        url.to_file_path()
            .expect("security-scoped URL must be a local file URL");
        let scoped = unsafe { url.startAccessingSecurityScopedResource() };
        Self { url, scoped }
    }
}

impl Drop for ScopedUrl {
    fn drop(&mut self) {
        if self.scoped {
            // Balanced with the successful start; the retained URL outlives every media reader.
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

#[derive(Default)]
pub struct Imports {
    queue: ImportQueue,
    pub(super) source_deletion_open: bool,
    urls: Vec<ScopedUrl>,
    pending_urls: Vec<(
        shrimply_timeline_skia::import_queue::BatchId,
        Vec<ScopedUrl>,
    )>,
}

pub enum Destination {
    Timeline(Placement),
    Tracks(Vec<TrackKey>, shrimply_math_core::Time),
}

impl Imports {
    pub(super) fn retain_scopes(&self) -> Vec<ScopedUrl> {
        self.urls
            .iter()
            .map(|scope| ScopedUrl::new(scope.url.clone()))
            .collect()
    }

    pub fn enqueue(
        &mut self,
        urls: impl IntoIterator<Item = Retained<NSURL>>,
        session: &EditorSession,
        destination: Destination,
    ) -> Result<ImportStart, String> {
        let (paths, scopes) = scoped_file_urls(urls)?;
        let duration = preferences::snapshot(&session.preferences).default_visual_duration;
        let project = session.project.borrow();
        let started = match destination {
            Destination::Timeline(placement) => {
                self.queue.enqueue(paths, &project, placement, duration)?
            }
            Destination::Tracks(tracks, position) => self
                .queue
                .enqueue_tracks(paths, &project, &tracks, position, duration)?,
        };
        self.pending_urls.push((started.batch, scopes));
        Ok(started)
    }

    pub(super) fn retain_pending(
        &mut self,
        batch: shrimply_timeline_skia::import_queue::BatchId,
        scopes: Vec<ScopedUrl>,
    ) {
        self.pending_urls.push((batch, scopes));
    }

    pub(super) fn finish_external(
        &mut self,
        event: shrimply_timeline_skia::external_content::ExternalImportEvent,
    ) {
        self.finish_scopes(event.batch, event.retained_paths.is_some());
    }

    pub fn poll(&mut self, session: &EditorSession) -> Result<(), String> {
        loop {
            let completion = self.queue.poll(&mut session.project.borrow_mut());
            let Some(completion) = completion else {
                return Ok(());
            };
            let succeeded = completion.result.is_ok();
            let batch = completion.batch;
            if completion.cancelled {
                self.finish_scopes(batch, false);
                continue;
            }
            let result = import::finish_track_import(
                &session.player_state,
                &session.selection_state,
                completion.result,
            );
            self.finish_scopes(batch, succeeded);
            result?;
        }
    }

    pub(super) fn take_source_deletion(&mut self) -> Option<SourceDeletion> {
        if self.source_deletion_open {
            return None;
        }
        self.queue.take_source_deletion()
    }

    fn finish_scopes(&mut self, batch: shrimply_timeline_skia::import_queue::BatchId, keep: bool) {
        let Some(index) = self
            .pending_urls
            .iter()
            .position(|(pending, _)| *pending == batch)
        else {
            return;
        };
        let (_, scopes) = self.pending_urls.swap_remove(index);
        if keep {
            // Remuxed imports still need access to the original for the deletion prompt.
            self.urls.extend(scopes);
        }
    }
}

pub(super) fn confirm_source_deletion(
    request: SourceDeletion,
    imports: &std::cell::RefCell<Imports>,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    imports.borrow_mut().source_deletion_open = true;
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Delete original file?"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "The file was remuxed. Delete the original to keep only the MP4 copy?\n\n{}",
        request.source().display(),
    )));
    alert.addButtonWithTitle(&NSString::from_str("Keep"));
    alert.addButtonWithTitle(&NSString::from_str("Delete"));
    let delete = alert.runModal() == objc2_app_kit::NSAlertSecondButtonReturn;
    imports.borrow_mut().source_deletion_open = false;
    if delete {
        request.delete()?;
    }
    Ok(())
}

pub(super) fn scoped_file_urls(
    urls: impl IntoIterator<Item = Retained<NSURL>>,
) -> Result<(Vec<PathBuf>, Vec<ScopedUrl>), String> {
    let urls: Vec<_> = urls.into_iter().collect();
    let paths = file_url_paths(&urls)?;
    let scopes = urls.into_iter().map(ScopedUrl::new).collect();
    Ok((paths, scopes))
}

pub fn file_url_paths(urls: &[Retained<NSURL>]) -> Result<Vec<PathBuf>, String> {
    let paths = urls
        .iter()
        .map(|url| {
            url.to_file_path()
                .ok_or_else(|| "only local files can be imported".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.is_empty() {
        return Err("drop contains no files".into());
    }
    Ok(paths)
}

pub fn file_urls(pasteboard: &NSPasteboard) -> Vec<Retained<NSURL>> {
    // Request NSURL objects so AppKit performs percent decoding and preserves file-system paths.
    unsafe {
        pasteboard.readObjectsForClasses_options(&NSArray::from_slice(&[NSURL::class()]), None)
    }
    .map(|objects| {
        objects
            .iter()
            .filter_map(|object| object.downcast::<NSURL>().ok())
            .filter(|url| url.isFileURL())
            .collect()
    })
    .unwrap_or_default()
}

pub fn stage_clipboard_file_urls(
    urls: Vec<Retained<NSURL>>,
) -> Result<Vec<Retained<NSURL>>, String> {
    urls.into_iter()
        .map(|url| {
            let path = url
                .to_file_path()
                .ok_or_else(|| "only local clipboard files can be imported".to_string())?;
            let scope = ScopedUrl::new(url.clone());
            let stored =
                shrimply_timeline_skia::external_content::store_clipboard_visual_file(&path)?;
            drop(scope);
            stored.map_or(Ok(url), |path| {
                NSURL::from_file_path(path)
                    .ok_or_else(|| "Could not resolve the stored clipboard visual".to_string())
            })
        })
        .collect()
}

pub fn clipboard_image_path(pasteboard: &NSPasteboard) -> Result<Option<PathBuf>, String> {
    let data = if let Some(data) = pasteboard.dataForType(unsafe { NSPasteboardTypePNG }) {
        data
    } else {
        let Some(tiff) = pasteboard.dataForType(unsafe { NSPasteboardTypeTIFF }) else {
            return Ok(None);
        };
        shrimply_timeline_skia::external_content::validate_clipboard_image_length(tiff.length())?;
        let image = NSBitmapImageRep::imageRepWithData(&tiff)
            .ok_or("clipboard TIFF image cannot be decoded")?;
        unsafe {
            image.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }
        .ok_or("clipboard image cannot be encoded as PNG")?
    };
    let bytes = unsafe { data.as_bytes_unchecked() };
    shrimply_timeline_skia::external_content::store_clipboard_image(bytes).map(Some)
}

pub fn choose_files(
    imports: &std::cell::RefCell<Imports>,
    session: &EditorSession,
    tracks: &[TrackKey],
    position: shrimply_math_core::Time,
    mtm: MainThreadMarker,
) -> Result<(), String> {
    let addresses = {
        let project = session.project.borrow();
        tracks
            .iter()
            .map(|key| {
                shrimply_timeline_skia::selection_state::track_address(&project, *key)
                    .ok_or("import destination track no longer exists")
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    if panel.runModal() != NSModalResponseOK {
        return Ok(());
    }
    let tracks = {
        let project = session.project.borrow();
        addresses
            .iter()
            .map(|address| {
                shrimply_timeline_skia::selection_state::track_key(&project, address)
                    .ok_or("import destination track was removed while choosing files")
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let started = imports.borrow_mut().enqueue(
        panel.URLs(),
        session,
        if tracks.is_empty() {
            Destination::Timeline(Placement {
                start: position,
                target: NewItemTarget::Automatic,
                collision: DragCollisionMode::NewTrack,
            })
        } else {
            Destination::Tracks(tracks.to_vec(), position)
        },
    )?;
    if started.needs_remux {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Remux MKV/WebM to MP4?"));
        alert.setInformativeText(&NSString::from_str(
            "MP4 is the supported timeline format. The MP4 will be created beside the source. After import, you can choose whether to delete the original.",
        ));
        alert.addButtonWithTitle(&NSString::from_str("Remux"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));
        let accepted = alert.runModal() == NSAlertFirstButtonReturn;
        imports
            .borrow_mut()
            .queue
            .confirm_remux(started.batch, accepted)?;
    }
    Ok(())
}
