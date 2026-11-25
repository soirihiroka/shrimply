use super::*;
use block2::RcBlock;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSModalResponseStop, NSProgressIndicator};
use objc2_foundation::{NSPoint, ns_string};
use shrimply_export_core::{
    job::{Event, Job, Outcome},
    video::progress,
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Instant};

const ACCESSORY_WIDTH: f64 = 320.0;
const PROGRESS_HEIGHT: f64 = 20.0;

pub(super) struct MediaExport {
    job: Job,
    alert: Retained<NSAlert>,
    progress: Retained<NSProgressIndicator>,
    pub destination: PathBuf,
    pub started: Instant,
}

impl CanvasView {
    pub(super) fn show_export_progress(
        &self,
        slot: &RefCell<Option<MediaExport>>,
        alert: Retained<NSAlert>,
        destination: PathBuf,
        job: Job,
    ) {
        alert.addButtonWithTitle(ns_string!("Cancel"));
        let progress = NSProgressIndicator::initWithFrame(
            NSProgressIndicator::alloc(self.mtm()),
            NSRect::new(NSPoint::ZERO, NSSize::new(ACCESSORY_WIDTH, PROGRESS_HEIGHT)),
        );
        progress.setIndeterminate(false);
        progress.setMinValue(0.0);
        progress.setMaxValue(1.0);
        alert.setAccessoryView(Some(&progress));
        let cancelled = job.cancellation_flag();
        let completion = RcBlock::new(move |response| {
            if response == NSAlertFirstButtonReturn {
                cancelled.store(true, Ordering::Release);
            }
        });
        slot.replace(Some(MediaExport {
            job,
            alert: alert.clone(),
            progress,
            destination,
            started: Instant::now(),
        }));
        alert.beginSheetModalForWindow_completionHandler(
            &self.window().expect("canvas must be attached"),
            Some(&completion),
        );
    }
}

pub(super) fn poll_export(
    slot: &RefCell<Option<MediaExport>>,
) -> Result<Option<MediaExport>, String> {
    let outcome = {
        let mut active = slot.borrow_mut();
        let Some(task) = active.as_mut() else {
            return Ok(None);
        };
        task.job.poll().find_map(|event| match event {
            Event::Progress(event) => {
                let event = progress::presentation(event);
                let label = shrimply_i18n::text_args(
                    event.message,
                    &event.arguments.into_iter().collect::<Vec<_>>(),
                );
                task.alert.setInformativeText(&NSString::from_str(&label));
                if let Some(fraction) = event.fraction {
                    task.progress.setDoubleValue(fraction);
                }
                None
            }
            Event::Finished(outcome) => Some(outcome),
            Event::Started | Event::Cancelling => None,
        })
    };
    let Some(outcome) = outcome else {
        return Ok(None);
    };
    let task = slot.borrow_mut().take().expect("export is active");
    if let Some(parent) = task.alert.window().sheetParent() {
        parent.endSheet_returnCode(&task.alert.window(), NSModalResponseStop);
    }
    match outcome {
        Outcome::Complete(()) => Ok(Some(task)),
        Outcome::Cancelled => Ok(None),
        Outcome::Failed(error) => Err(error),
    }
}
