use super::super::media::ScopedUrl;
use super::*;
use block2::RcBlock;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSModalResponseStop, NSPopUpButton,
    NSProgressIndicator, NSSavePanel,
};
use objc2_foundation::{NSPoint, NSString, ns_string};
use objc2_uniform_type_identifiers::UTType;
use shrimply_export_core::audio::{self, ExportProgress, Format};
use shrimply_timeline_core::{audio_selection::selected_audio_project, selection_state};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
};

const ACCESSORY_WIDTH: f64 = 320.0;
const FORMAT_HEIGHT: f64 = 32.0;
const PROGRESS_HEIGHT: f64 = 20.0;
const FORMATS: [(&str, Format); 5] = [
    ("WAV", Format::Wav),
    ("FLAC", Format::Flac),
    ("MP3", Format::Mp3),
    ("OGG Vorbis", Format::Ogg),
    ("Opus", Format::Opus),
];

enum Event {
    Progress(ExportProgress),
    Finished(Result<(), String>),
}

pub(super) struct AudioExport {
    receiver: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    alert: Retained<NSAlert>,
    progress: Retained<NSProgressIndicator>,
    destination: PathBuf,
}

impl Drop for AudioExport {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl CanvasView {
    pub(super) fn export_selected_audio(&self) -> Result<(), String> {
        if self.ivars().audio_export.borrow().is_some() {
            return Err("An audio export is already running.".into());
        }
        let original = self.ivars().session.project.borrow().clone();
        let mut selection = selected_audio_project(
            &original,
            &selection_state::selected_items(&self.ivars().session.selection_state),
            &selection_state::selected_tracks(&self.ivars().session.selection_state),
        )
        .ok_or("No audio is selected.")?;
        for track in &mut selection.project.audio_tracks {
            for item in &mut track.items {
                item.start = item.start.saturating_sub(selection.start);
                item.end = item.end.saturating_sub(selection.start);
            }
        }

        let format_control = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(ACCESSORY_WIDTH, FORMAT_HEIGHT)),
            false,
        );
        for (label, _) in FORMATS {
            format_control.addItemWithTitle(&NSString::from_str(label));
        }
        let format_alert = NSAlert::new(self.mtm());
        format_alert.setMessageText(ns_string!("Export Selected Audio"));
        format_alert.setInformativeText(ns_string!(
            "The selected items will be mixed into one audio file."
        ));
        format_alert.setAccessoryView(Some(&format_control));
        format_alert.addButtonWithTitle(ns_string!("Choose File"));
        format_alert.addButtonWithTitle(ns_string!("Cancel"));
        if format_alert.runModal() != NSAlertFirstButtonReturn {
            return Ok(());
        }
        let index = usize::try_from(format_control.indexOfSelectedItem())
            .map_err(|_| "No audio format is selected.")?;
        let (_, format) = *FORMATS.get(index).ok_or("Unknown audio export format.")?;
        let content_type =
            UTType::typeWithFilenameExtension(&NSString::from_str(format.extension()))
                .ok_or("macOS does not recognize the selected audio format.")?;
        let panel = NSSavePanel::savePanel(self.mtm());
        panel.setTitle(Some(ns_string!("Export Selected Audio")));
        panel.setNameFieldStringValue(&NSString::from_str(&format!(
            "selected-audio.{}",
            format.extension()
        )));
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&[content_type]));
        panel.setAllowsOtherFileTypes(false);
        panel.setCanCreateDirectories(true);
        if panel.runModal() != NSModalResponseOK {
            return Ok(());
        }
        let url = panel
            .URL()
            .ok_or("The save panel returned no destination.")?;
        let destination = url
            .to_file_path()
            .ok_or("Audio must be saved to a local file.")?;
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Exporting Audio"));
        alert.setInformativeText(ns_string!("Preparing audio"));
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(ACCESSORY_WIDTH, PROGRESS_HEIGHT)),
        );
        progress.setIndeterminate(false);
        progress.setMinValue(0.0);
        progress.setMaxValue(1.0);
        alert.setAccessoryView(Some(&progress));
        let cancelled = Arc::new(AtomicBool::new(false));
        let completion = RcBlock::new({
            let cancelled = cancelled.clone();
            move |response| {
                if response == NSAlertFirstButtonReturn {
                    cancelled.store(true, Ordering::Relaxed);
                }
            }
        });
        // The single slot bounds progress traffic while retaining the final result.
        let (sender, receiver) = mpsc::sync_channel(1);
        let state = AudioExport {
            receiver,
            cancelled: cancelled.clone(),
            alert: alert.clone(),
            progress,
            destination: destination.clone(),
        };
        let destination_scope = ScopedUrl::new(url);
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        std::thread::Builder::new()
            .name("audio-export".into())
            .spawn(move || {
                let _destination_scope = destination_scope;
                let _source_scopes = source_scopes;
                let result = (|| {
                    shrimply_export_core::ensure_output_is_not_an_asset(&original, &destination)?;
                    let parent = destination
                        .parent()
                        .ok_or("Export destination has no directory.")?;
                    let temporary = tempfile::Builder::new()
                        .prefix(".shrimply-audio-")
                        .suffix(&format!(".{}", format.extension()))
                        .tempfile_in(parent)
                        .map_err(|error| format!("Could not prepare audio export: {error}"))?;
                    audio::export_with_progress(
                        &selection.project,
                        temporary.path(),
                        format,
                        |progress| {
                            let _ = sender.try_send(Event::Progress(progress));
                            !cancelled.load(Ordering::Relaxed)
                        },
                    )?;
                    if cancelled.load(Ordering::Relaxed) {
                        return Err("Export cancelled".into());
                    }
                    temporary
                        .persist(&destination)
                        .map_err(|error| format!("Could not save audio export: {error}"))?;
                    Ok(())
                })();
                let _ = sender.send(Event::Finished(result));
            })
            .map_err(|error| format!("Could not start audio export: {error}"))?;
        self.ivars().audio_export.replace(Some(state));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    pub(super) fn poll_audio_export(&self) -> Result<(), String> {
        let finished = {
            let mut export = self.ivars().audio_export.borrow_mut();
            let Some(task) = export.as_mut() else {
                return Ok(());
            };
            match task.receiver.try_recv() {
                Ok(Event::Progress(progress)) => {
                    let (label, completed, total) = match progress {
                        ExportProgress::Mixing {
                            completed_frames,
                            total_frames,
                        } => ("Preparing audio", completed_frames, total_frames),
                        ExportProgress::Encoding {
                            completed_frames,
                            total_frames,
                        } => ("Encoding audio", completed_frames, total_frames),
                    };
                    task.alert.setInformativeText(&NSString::from_str(label));
                    task.progress.setDoubleValue(if total == 0 {
                        1.0
                    } else {
                        (completed as f64 / total as f64).clamp(0.0, 1.0)
                    });
                    return Ok(());
                }
                Ok(Event::Finished(result)) => result,
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => {
                    Err("The audio export worker stopped unexpectedly.".into())
                }
            }
        };
        let task = self
            .ivars()
            .audio_export
            .borrow_mut()
            .take()
            .expect("audio export is active");
        if let Some(parent) = task.alert.window().sheetParent() {
            parent.endSheet_returnCode(&task.alert.window(), NSModalResponseStop);
        }
        if task.cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        finished?;
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(ns_string!("Audio exported"));
        alert.setInformativeText(&NSString::from_str(&task.destination.display().to_string()));
        alert.addButtonWithTitle(ns_string!("OK"));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            None,
        );
        Ok(())
    }
}
