use block2::RcBlock;
use objc2_app_kit::{NSApplication, NSModalResponseCancel, NSModalResponseOK, NSOpenPanel};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use objc2_uniform_type_identifiers::UTType;
use shrimply_inspector_core::{InspectorControlAction, file_selection::FileSelection};

use super::control::Context;

pub(super) fn select(
    context: &Context,
    action: InspectorControlAction,
    selection: FileSelection,
    mtm: MainThreadMarker,
) {
    context.end_editing();
    let context = context.clone();
    choose(
        selection,
        move |result| {
            let result = result.and_then(|path| {
                path.map_or(Ok(()), |path| {
                    context
                        .controller
                        .set_control_file(&context.target, action, &path)
                })
            });
            context.refresh(result);
        },
        mtm,
    );
}

pub(super) fn choose(
    selection: FileSelection,
    on_selected: impl Fn(Result<Option<std::path::PathBuf>, String>) + 'static,
    mtm: MainThreadMarker,
) {
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setTitle(Some(&NSString::from_str(selection.title)));
    panel.setMessage(Some(&NSString::from_str(selection.title)));
    panel.setCanChooseDirectories(false);
    panel.setCanChooseFiles(true);
    panel.setAllowsMultipleSelection(false);
    let types = selection
        .extensions
        .iter()
        .map(|extension| {
            UTType::typeWithFilenameExtension(&NSString::from_str(extension))
                .expect("inspector file extension must have a content type")
        })
        .collect::<Vec<_>>();
    if !types.is_empty() {
        panel.setAllowedContentTypes(&NSArray::from_retained_slice(&types));
    }
    let selected_panel = panel.clone();
    let completion = RcBlock::new(move |response| {
        selected_panel.orderOut(None);
        if response == NSModalResponseCancel {
            return;
        }
        if response != NSModalResponseOK {
            on_selected(Err(
                "the inspector file picker could not be displayed".into()
            ));
            return;
        }
        let result = selected_panel
            .URL()
            .and_then(|url| url.to_file_path())
            .ok_or_else(|| "selected inspector file has no local path".to_string())
            .map(Some);
        on_selected(result);
    });
    if let Some(window) = NSApplication::sharedApplication(mtm).keyWindow() {
        panel.beginSheetModalForWindow_completionHandler(&window, &completion);
    } else {
        panel.beginWithCompletionHandler(&completion);
    }
}
