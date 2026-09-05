use super::super::media::ScopedUrl;
use super::*;
use block2::RcBlock;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSModalResponseStop, NSPasteboard,
    NSPasteboardTypePNG, NSSavePanel,
};
use objc2_foundation::{
    NSData, NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask, NSString, NSURL,
    ns_string,
};
use objc2_uniform_type_identifiers::UTTypePNG;
use shrimply_timeline_core::{VideoFrameSelection, video_selection};
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

enum Destination {
    Clipboard,
    File(PathBuf, ScopedUrl),
}

enum Source {
    Presented(skia_safe::Image),
    Selected {
        project: Box<shrimply_project::project::Project>,
        position: shrimply_math_core::Time,
    },
}

enum Captured {
    Clipboard(Vec<u8>),
    File,
}

pub(super) struct FrameCapture {
    receiver: Receiver<Result<Captured, String>>,
    cancelled: Arc<AtomicBool>,
    alert: Retained<NSAlert>,
}

impl Drop for FrameCapture {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl CanvasView {
    pub(super) fn capture_selected_frame(
        &self,
        selection: VideoFrameSelection,
        save: bool,
    ) -> Result<(), String> {
        let session = &self.ivars().session;
        let (original, position, item_ids) = video_selection::prepare_selected_video_frame(
            &session.project,
            &session.player_state,
            &session.selection_state,
            selection,
        );
        let project = video_selection::selected_video_project(&original, &item_ids)?;
        self.capture_frame(
            original,
            Source::Selected {
                project: Box::new(project),
                position,
            },
            save,
        )
    }

    pub(super) fn capture_preview_image(&self, save: bool) -> Result<(), String> {
        let image = {
            let content = self.ivars().content.borrow();
            let Content::Preview(state) = &*content else {
                return Ok(());
            };
            state.renderer.image().cloned()
        };
        let Some(image) = image else {
            return Ok(());
        };
        // Snapshot the displayed frame before opening a picker or yielding to playback.
        let original = self.ivars().session.project.borrow().clone();
        self.capture_frame(original, Source::Presented(image), save)
    }

    fn capture_frame(
        &self,
        original: shrimply_project::project::Project,
        source: Source,
        save: bool,
    ) -> Result<(), String> {
        if self.ivars().frame_capture.borrow().is_some() {
            return Err("A frame capture is already running.".into());
        }
        let preview = matches!(&source, Source::Presented(_));
        let destination = if save {
            let panel = NSSavePanel::savePanel(self.mtm());
            panel.setTitle(Some(&NSString::from_str(if preview {
                "Save Preview Image"
            } else {
                "Save Selected Frame"
            })));
            panel.setNameFieldStringValue(&NSString::from_str(if preview {
                "preview.png"
            } else {
                "frame.png"
            }));
            if preview {
                let folder = shrimply_state::preferences::preview_image_folder(
                    &self.ivars().session.preferences,
                )
                .and_then(|path| NSURL::from_file_path(&path))
                .or_else(|| {
                    NSFileManager::defaultManager()
                        .URLsForDirectory_inDomains(
                            NSSearchPathDirectory::PicturesDirectory,
                            NSSearchPathDomainMask::UserDomainMask,
                        )
                        .firstObject()
                });
                panel.setDirectoryURL(folder.as_deref());
            }
            panel.setAllowedContentTypes(&NSArray::from_slice(&[unsafe { UTTypePNG }]));
            panel.setAllowsOtherFileTypes(false);
            panel.setCanCreateDirectories(true);
            if panel.runModal() != NSModalResponseOK {
                return Ok(());
            }
            let url = panel
                .URL()
                .ok_or("The save panel returned no destination.")?;
            let mut path = url
                .to_file_path()
                .ok_or("Frames must be saved to a local file.")?;
            if preview {
                if !path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                {
                    path.set_extension("png");
                }
                if let Some(folder) = path.parent() {
                    shrimply_state::preferences::set_preview_image_folder(
                        &self.ivars().session.preferences,
                        folder,
                    );
                }
            }
            Destination::File(path, ScopedUrl::new(url))
        } else {
            Destination::Clipboard
        };
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(if preview {
            "Preparing Preview Image"
        } else {
            "Rendering Selected Frame"
        }));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let cancelled = Arc::new(AtomicBool::new(false));
        let completion = RcBlock::new({
            let cancelled = Arc::clone(&cancelled);
            move |response| {
                if response == NSAlertFirstButtonReturn {
                    cancelled.store(true, Ordering::Relaxed);
                }
            }
        });
        let (sender, receiver) = mpsc::sync_channel(1);
        let task = FrameCapture {
            receiver,
            cancelled: Arc::clone(&cancelled),
            alert: alert.clone(),
        };
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        std::thread::Builder::new()
            .name("frame-capture".into())
            .spawn(move || {
                let _source_scopes = source_scopes;
                let result = (|| {
                    if let Destination::File(path, _) = &destination {
                        shrimply_export_core::ensure_output_is_not_an_asset(&original, path)?;
                    }
                    if cancelled.load(Ordering::Relaxed) {
                        return Err("Frame capture cancelled".into());
                    }
                    let png = match source {
                        Source::Selected { project, position } => {
                            shrimply_preview_metal::render_png(&project, position)?
                        }
                        Source::Presented(image) => {
                            skia_safe::png_encoder::encode_image(None, &image, &Default::default())
                                .ok_or("Could not encode the preview image")?
                                .as_bytes()
                                .to_vec()
                        }
                    };
                    if cancelled.load(Ordering::Relaxed) {
                        return Err("Frame capture cancelled".into());
                    }
                    match destination {
                        Destination::Clipboard => Ok(Captured::Clipboard(png)),
                        Destination::File(path, _scope) => {
                            let parent =
                                path.parent().ok_or("Frame destination has no directory.")?;
                            let mut temporary = tempfile::Builder::new()
                                .prefix(".shrimply-frame-")
                                .suffix(".png")
                                .tempfile_in(parent)
                                .map_err(|error| {
                                    format!("Could not prepare frame export: {error}")
                                })?;
                            temporary
                                .write_all(&png)
                                .and_then(|()| temporary.as_file().sync_all())
                                .map_err(|error| {
                                    format!("Could not write the selected frame: {error}")
                                })?;
                            if cancelled.load(Ordering::Relaxed) {
                                return Err("Frame capture cancelled".into());
                            }
                            temporary.persist(path).map_err(|error| {
                                format!("Could not save the selected frame: {error}")
                            })?;
                            Ok(Captured::File)
                        }
                    }
                })();
                let _ = sender.send(result);
            })
            .map_err(|error| format!("Could not start frame capture: {error}"))?;
        self.ivars().frame_capture.replace(Some(task));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    pub(super) fn poll_frame_capture(&self) -> Result<(), String> {
        let result = {
            let state = self.ivars().frame_capture.borrow();
            let Some(task) = state.as_ref() else {
                return Ok(());
            };
            match task.receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    Err("The frame renderer stopped unexpectedly.".into())
                }
            }
        };
        let task = self
            .ivars()
            .frame_capture
            .borrow_mut()
            .take()
            .expect("frame capture is active");
        if let Some(parent) = task.alert.window().sheetParent() {
            parent.endSheet_returnCode(&task.alert.window(), NSModalResponseStop);
        }
        if task.cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        if let Captured::Clipboard(png) = result? {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            if !pasteboard.setData_forType(Some(&NSData::with_bytes(&png)), unsafe {
                NSPasteboardTypePNG
            }) {
                return Err("Could not copy the selected frame to the clipboard.".into());
            }
        }
        Ok(())
    }
}
