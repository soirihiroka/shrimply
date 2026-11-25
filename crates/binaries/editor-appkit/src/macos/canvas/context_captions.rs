use super::super::media::ScopedUrl;
use super::*;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSModalResponseOK, NSSavePanel, NSWorkspace,
};
use objc2_foundation::ns_string;
use objc2_uniform_type_identifiers::{UTType, UTTypeText};
use shrimply_caption_export_core::{prepare_export, write_export};
use shrimply_export_core::job::{self, Event, Outcome};

impl CanvasView {
    pub(in crate::macos) fn export_captions(&self) -> Result<(), String> {
        let project = self.ivars().session.project.borrow().clone();
        let Some(settings) = shrimply_export_appkit::choose_caption_settings(
            &self.window().expect("canvas must be attached"),
        ) else {
            return Ok(());
        };
        let extension = settings.format.extension();
        let content_type = UTType::typeWithFilenameExtension_conformingToType(
            &NSString::from_str(extension),
            unsafe { UTTypeText },
        )
        .ok_or("macOS could not create a subtitle file type.")?;
        let panel = NSSavePanel::savePanel(self.mtm());
        panel.setTitle(Some(ns_string!("Export Captions")));
        panel.setNameFieldStringValue(&NSString::from_str(
            &shrimply_export_core::output::default_filename(&project, extension),
        ));
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
            .ok_or("Captions must be saved to a local file.")?;
        let _scope = ScopedUrl::new(url);
        let outputs = prepare_export(&project, &destination, settings)?;
        // The native panel confirms its selected path. Separate tracks and any
        // normalized extension can produce different paths that need confirmation.
        let replacements = outputs
            .iter()
            .filter(|output| output.path != destination && output.path.exists())
            .map(|output| output.path.display().to_string())
            .collect::<Vec<_>>();
        if !replacements.is_empty() {
            let alert = NSAlert::new(self.mtm());
            alert.setMessageText(ns_string!("Replace existing caption files?"));
            alert.setInformativeText(&NSString::from_str(&replacements.join("\n")));
            alert.addButtonWithTitle(ns_string!("Cancel"));
            alert.addButtonWithTitle(ns_string!("Replace"));
            if alert.runModal() != objc2_app_kit::NSAlertSecondButtonReturn {
                return Ok(());
            }
        }
        let mut result = Ok(());
        job::run(
            Default::default(),
            |_, _| write_export(outputs),
            |event| {
                let paths = match event {
                    Event::Finished(Outcome::Complete(paths)) => paths,
                    Event::Finished(Outcome::Failed(error)) => {
                        result = Err(error);
                        return;
                    }
                    _ => return,
                };
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(ns_string!("Captions exported"));
                alert.setInformativeText(&NSString::from_str(&format!(
                    "{} caption file(s) exported.\n{}",
                    paths.len(),
                    paths
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                )));
                alert.addButtonWithTitle(ns_string!("Show in Finder"));
                alert.addButtonWithTitle(ns_string!("Close"));
                if alert.runModal() == NSAlertFirstButtonReturn {
                    let urls = paths
                        .iter()
                        .map(|path| {
                            NSURL::from_file_path(path).expect("exported paths are valid file URLs")
                        })
                        .collect::<Vec<_>>();
                    NSWorkspace::sharedWorkspace()
                        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&urls));
                }
            },
        );
        result
    }
}
