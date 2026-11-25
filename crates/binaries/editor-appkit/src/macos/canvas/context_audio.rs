use super::super::media::ScopedUrl;
use super::{export_progress::poll_export, *};
use objc2_app_kit::{NSAlert, NSModalResponseOK, NSSavePanel};
use objc2_foundation::ns_string;
use objc2_uniform_type_identifiers::UTType;
use shrimply_export_core::{audio, job::Job};
use shrimply_timeline_skia::{audio_selection::selected_audio_project, selection_state};
use std::sync::atomic::Ordering;

impl CanvasView {
    pub(super) fn export_selected_audio(&self) -> Result<(), String> {
        if self.ivars().audio_export.borrow().is_some() {
            return Err("An audio export is already running.".into());
        }
        let original = self.ivars().session.project.borrow().clone();
        let project = selected_audio_project(
            &original,
            &selection_state::selected_items(&self.ivars().session.selection_state),
            &selection_state::selected_tracks(&self.ivars().session.selection_state),
        )
        .ok_or("No audio is selected.")?
        .at_zero();

        let Some(format) = shrimply_export_appkit::choose_audio_format(
            &self.window().expect("canvas must be attached"),
        ) else {
            return Ok(());
        };
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
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        let destination_scope = ScopedUrl::new(url);
        let path = destination.clone();
        let job = Job::spawn(
            move |cancelled, progress| {
                let _source_scopes = source_scopes;
                let _destination_scope = destination_scope;
                shrimply_export_core::ensure_output_is_not_an_asset(&original, &path)?;
                let parent = path
                    .parent()
                    .ok_or("Export destination has no directory.")?;
                let temporary = tempfile::Builder::new()
                    .prefix(".shrimply-audio-")
                    .suffix(&format!(".{}", format.extension()))
                    .tempfile_in(parent)
                    .map_err(|error| format!("Could not prepare audio export: {error}"))?;
                audio::export_with_progress(&project, temporary.path(), format, |update| {
                    progress(update);
                    !cancelled.load(Ordering::Acquire)
                })?;
                if cancelled.load(Ordering::Acquire) {
                    return Err("Export cancelled".into());
                }
                temporary
                    .persist(&path)
                    .map_err(|error| format!("Could not save audio export: {error}"))?;
                Ok(())
            },
            || {},
        )?;
        self.show_export_progress(&self.ivars().audio_export, alert, destination, job);
        Ok(())
    }

    pub(super) fn poll_audio_export(&self) -> Result<(), String> {
        let Some(task) = poll_export(&self.ivars().audio_export)? else {
            return Ok(());
        };
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
