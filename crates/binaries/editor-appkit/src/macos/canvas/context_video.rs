use super::super::media::ScopedUrl;
use super::*;
use block2::RcBlock;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSModalResponseStop, NSProgressIndicator,
    NSSavePanel, NSWorkspace,
};
use objc2_foundation::{NSArray, NSPoint, NSString, NSURL, ns_string};
use shrimply_export_core::video::ExportProgress;
use shrimply_export_metal as metal;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Instant,
};

const ACCESSORY_WIDTH: f64 = 320.0;
const PROGRESS_HEIGHT: f64 = 20.0;

enum Event {
    Progress(ExportProgress),
    Finished(Result<(), String>),
}

pub(super) struct VideoExport {
    receiver: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    alert: Retained<NSAlert>,
    progress: Retained<NSProgressIndicator>,
    destination: PathBuf,
    started: Instant,
}

impl Drop for VideoExport {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl CanvasView {
    pub(in crate::macos) fn export_video(&self) -> Result<(), String> {
        if self.ivars().video_export.borrow().is_some() {
            return Err("A video export is already running.".into());
        }
        let project = self.ivars().session.project.borrow().clone();
        let Some(mut settings) = shrimply_export_appkit::choose_settings(
            &self.window().expect("canvas must be attached"),
            &project,
            shrimply_editor_state::preferences::snapshot(&self.ivars().session.preferences)
                .temporal_decoder_pool_size as usize,
        ) else {
            return Ok(());
        };
        let extension = shrimply_export_core::video::extension_for_container(settings.container);
        let project_name = project.name.trim();
        let project_name = if project_name.is_empty() {
            "Untitled"
        } else {
            project_name
        };
        let panel = NSSavePanel::savePanel(self.mtm());
        panel.setTitle(Some(ns_string!("Export Video")));
        panel.setNameFieldStringValue(&NSString::from_str(&format!("{project_name}.{extension}")));
        panel.setCanCreateDirectories(true);
        if panel.runModal() != NSModalResponseOK {
            return Ok(());
        }
        let url = panel
            .URL()
            .ok_or("The save panel returned no destination.")?;
        let destination = url
            .to_file_path()
            .ok_or("Video must be saved to a local file.")?;
        settings.path = destination.clone();
        let alert = NSAlert::new(self.mtm());
        // `nil` is explicitly supported and suppresses the application icon.
        unsafe { alert.setIcon(None) };
        alert.setMessageText(ns_string!("Exporting Video"));
        alert.setInformativeText(ns_string!("Preparing Metal renderer"));
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
        let original = project;
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        let destination_scope = ScopedUrl::new(url);
        let (sender, receiver) = mpsc::channel();
        let state = VideoExport {
            receiver,
            cancelled: cancelled.clone(),
            alert: alert.clone(),
            progress,
            destination: destination.clone(),
            started: Instant::now(),
        };
        std::thread::Builder::new()
            .name("metal-video-export".into())
            .spawn(move || {
                let _source_scopes = source_scopes;
                let _destination_scope = destination_scope;
                let result =
                    metal::export_project(original, settings, cancelled.clone(), |progress| {
                        let _ = sender.send(Event::Progress(progress));
                    });
                let _ = sender.send(Event::Finished(result));
            })
            .map_err(|error| format!("Could not start video export: {error}"))?;
        self.ivars().video_export.replace(Some(state));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
        Ok(())
    }

    pub(super) fn poll_video_export(&self) -> Result<(), String> {
        let finished = {
            let mut active = self.ivars().video_export.borrow_mut();
            let Some(task) = active.as_mut() else {
                return Ok(());
            };
            let mut latest_progress = None;
            let event = loop {
                match task.receiver.try_recv() {
                    Ok(Event::Progress(progress)) => latest_progress = Some(progress),
                    Ok(finished @ Event::Finished(_)) => break finished,
                    Err(TryRecvError::Empty) => match latest_progress {
                        Some(progress) => break Event::Progress(progress),
                        None => return Ok(()),
                    },
                    Err(TryRecvError::Disconnected) => {
                        break Event::Finished(Err(
                            "The video export worker stopped unexpectedly.".into(),
                        ));
                    }
                }
            };
            match event {
                Event::Progress(progress) => {
                    let (label, current, total) = match progress {
                        ExportProgress::MixingAudio {
                            current_frame,
                            total_frames,
                        } => {
                            let percent = percent(current_frame, total_frames);
                            (
                                format!("Preparing audio ({percent:.0}%)"),
                                current_frame,
                                total_frames,
                            )
                        }
                        ExportProgress::EncodingAudio {
                            current_frame,
                            total_frames,
                        } => {
                            let percent = percent(current_frame, total_frames);
                            (
                                format!("Encoding audio ({percent:.0}%)"),
                                current_frame,
                                total_frames,
                            )
                        }
                        ExportProgress::EncodingVideo {
                            current_frame,
                            total_frames,
                            fps_milli,
                        } => {
                            let percent = percent(current_frame, total_frames);
                            let detail = if fps_milli == 0 {
                                format!("{current_frame} of {total_frames} frames ({percent:.0}%)")
                            } else {
                                let remaining_ms = total_frames
                                    .saturating_sub(current_frame)
                                    .saturating_mul(1_000_000)
                                    .checked_div(fps_milli)
                                    .expect("positive export FPS");
                                let eta = shrimply_export_core::time_format::human_duration(
                                    std::time::Duration::from_millis(remaining_ms),
                                );
                                format!(
                                    "{current_frame} of {total_frames} frames ({percent:.0}%) — {}.{} fps — {eta} left",
                                    fps_milli / 1_000,
                                    fps_milli % 1_000 / 100
                                )
                            };
                            (detail, current_frame, total_frames)
                        }
                        ExportProgress::SettingUp(label) => {
                            task.alert.setInformativeText(&NSString::from_str(label));
                            return Ok(());
                        }
                        ExportProgress::Finalizing => {
                            task.alert
                                .setInformativeText(ns_string!("Finalizing video"));
                            return Ok(());
                        }
                    };
                    task.alert.setInformativeText(&NSString::from_str(&label));
                    task.progress.setDoubleValue(if total == 0 {
                        1.0
                    } else {
                        current as f64 / total as f64
                    });
                    return Ok(());
                }
                Event::Finished(result) => result,
            }
        };
        let task = self
            .ivars()
            .video_export
            .borrow_mut()
            .take()
            .expect("video export is active");
        if let Some(parent) = task.alert.window().sheetParent() {
            parent.endSheet_returnCode(&task.alert.window(), NSModalResponseStop);
        }
        if task.cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }
        finished?;
        let alert = NSAlert::new(self.mtm());
        // `nil` is explicitly supported and suppresses the application icon.
        unsafe { alert.setIcon(None) };
        alert.setMessageText(&NSString::from_str(&format!(
            "Video exported in {}",
            shrimply_export_core::time_format::human_duration(task.started.elapsed())
        )));
        alert.setInformativeText(&NSString::from_str(&task.destination.display().to_string()));
        alert.addButtonWithTitle(ns_string!("Show in Finder"));
        alert.addButtonWithTitle(ns_string!("Done"));
        let reveal = RcBlock::new(move |response| {
            if response == NSAlertFirstButtonReturn {
                let url = NSURL::fileURLWithPath(&NSString::from_str(
                    &task.destination.display().to_string(),
                ));
                NSWorkspace::sharedWorkspace()
                    .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
            }
        });
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&reveal),
        );
        Ok(())
    }
}

fn percent(current: u64, total: u64) -> f64 {
    if total == 0 {
        100.0
    } else {
        (current as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    }
}
