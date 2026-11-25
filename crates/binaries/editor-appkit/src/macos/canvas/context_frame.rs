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
use shrimply_export_core::job::{Event, Job, Outcome};
use shrimply_timeline_skia::{VideoFrameSelection, video_selection};
use std::{io::Write, path::PathBuf, sync::atomic::Ordering};

enum Destination {
    Clipboard,
    File(PathBuf, ScopedUrl),
}

enum Source {
    Presented(skia_safe::Image),
    Selected {
        project: Box<shrimply_project_document::project::Project>,
        position: shrimply_math_core::Time,
    },
}

enum Captured {
    Clipboard(Vec<u8>),
    File,
}

pub(super) struct FrameCapture {
    job: Job<Captured>,
    alert: Retained<NSAlert>,
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
            state.renderer.capture_image()?
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
        original: shrimply_project_document::project::Project,
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
                let folder = shrimply_editor_state::preferences::preview_image_folder(
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
                    shrimply_editor_state::preferences::set_preview_image_folder(
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
        let maximum_decoders =
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences)
                .temporal_decoder_pool_size as usize;
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        let job = Job::spawn(
            move |cancelled, _| {
                let _source_scopes = source_scopes;
                if let Destination::File(path, _) = &destination {
                    shrimply_export_core::ensure_output_is_not_an_asset(&original, path)?;
                }
                if cancelled.load(Ordering::Acquire) {
                    return Err("Frame capture cancelled".into());
                }
                let png = match source {
                    Source::Selected { project, position } => {
                        shrimply_preview_render_metal::render_png_cancelable(
                            &project,
                            position,
                            maximum_decoders,
                            &cancelled,
                        )?
                    }
                    Source::Presented(image) => {
                        skia_safe::png_encoder::encode_image(None, &image, &Default::default())
                            .ok_or("Could not encode the preview image")?
                            .as_bytes()
                            .to_vec()
                    }
                };
                if cancelled.load(Ordering::Acquire) {
                    return Err("Frame capture cancelled".into());
                }
                match destination {
                    Destination::Clipboard => Ok(Captured::Clipboard(png)),
                    Destination::File(path, _scope) => {
                        let parent = path.parent().ok_or("Frame destination has no directory.")?;
                        let mut temporary = tempfile::Builder::new()
                            .prefix(".shrimply-frame-")
                            .suffix(".png")
                            .tempfile_in(parent)
                            .map_err(|error| format!("Could not prepare frame export: {error}"))?;
                        temporary
                            .write_all(&png)
                            .and_then(|()| temporary.as_file().sync_all())
                            .map_err(|error| {
                                format!("Could not write the selected frame: {error}")
                            })?;
                        if cancelled.load(Ordering::Acquire) {
                            return Err("Frame capture cancelled".into());
                        }
                        temporary.persist(path).map_err(|error| {
                            format!("Could not save the selected frame: {error}")
                        })?;
                        Ok(Captured::File)
                    }
                }
            },
            || {},
        )?;
        let cancelled = job.cancellation_flag();
        let completion = RcBlock::new(move |response| {
            if response == NSAlertFirstButtonReturn {
                cancelled.store(true, Ordering::Release);
            }
        });
        self.ivars().frame_capture.replace(Some(FrameCapture {
            job,
            alert: alert.clone(),
        }));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    pub(super) fn poll_frame_capture(&self) -> Result<(), String> {
        let outcome = {
            let mut state = self.ivars().frame_capture.borrow_mut();
            let Some(task) = state.as_mut() else {
                return Ok(());
            };
            task.job.poll().find_map(|event| match event {
                Event::Finished(outcome) => Some(outcome),
                _ => None,
            })
        };
        let Some(outcome) = outcome else {
            return Ok(());
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
        if task.job.cancellation_flag().load(Ordering::Acquire) {
            return Ok(());
        }
        let captured = match outcome {
            Outcome::Complete(captured) => captured,
            Outcome::Cancelled => return Ok(()),
            Outcome::Failed(error) => return Err(error),
        };
        if let Captured::Clipboard(png) = captured {
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
