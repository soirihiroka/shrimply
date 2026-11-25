use super::super::media::ScopedUrl;
use super::{export_progress::poll_export, *};
use block2::RcBlock;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSSavePanel, NSWorkspace,
};
use objc2_foundation::{NSArray, NSString, NSURL, ns_string};
use shrimply_export_core::job::Job;
use shrimply_export_metal as metal;

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
        let source_scopes = self.ivars().imports.borrow().retain_scopes();
        let destination_scope = ScopedUrl::new(url);
        let job = Job::spawn(
            move |cancelled, progress| {
                let _source_scopes = source_scopes;
                let _destination_scope = destination_scope;
                metal::export_project(project, settings, cancelled, progress)
            },
            || {},
        )?;
        self.show_export_progress(&self.ivars().video_export, alert, destination, job);
        Ok(())
    }

    pub(super) fn poll_video_export(&self) -> Result<(), String> {
        let Some(task) = poll_export(&self.ivars().video_export)? else {
            return Ok(());
        };
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
