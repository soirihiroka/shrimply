use super::*;
use objc2_app_kit::NSScrollView;
use objc2_foundation::NSSize;
use shrimply_trust_core::{Kind, Review};

const DETAILS_SIZE: NSSize = NSSize::new(540.0, 220.0);

pub(super) fn confirm(review: &Review, mtm: MainThreadMarker) -> Option<Kind> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&format!(
        "Trust {} executable files?",
        review.files.len()
    )));
    alert.setInformativeText(&NSString::from_str(shrimply_trust_core::EXPLANATION));
    alert.addButtonWithTitle(ns_string!("Cancel"));
    alert.addButtonWithTitle(&NSString::from_str(&format!(
        "Trust {} files",
        review.files.len()
    )));
    alert.addButtonWithTitle(&NSString::from_str(&format!(
        "Trust {} folders",
        review.folders.len()
    )));
    let scroll = NSScrollView::initWithFrame(
        NSScrollView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, DETAILS_SIZE),
    );
    scroll.setHasVerticalScroller(true);
    scroll.setHasHorizontalScroller(true);
    let details = NSTextField::labelWithString(&NSString::from_str(&review.details()), mtm);
    details.setSelectable(true);
    details.sizeToFit();
    scroll.setDocumentView(Some(&details));
    alert.setAccessoryView(Some(&scroll));
    match alert.runModal() {
        response if response == NSAlertSecondButtonReturn => Some(Kind::File),
        response if response == NSAlertThirdButtonReturn => Some(Kind::Folder),
        _ => None,
    }
}

pub(super) fn manage(mtm: MainThreadMarker) -> Result<(), String> {
    loop {
        let entries = shrimply_trust_core::entries()?;
        let alert = NSAlert::new(mtm);
        alert.setMessageText(ns_string!("Trusted executable sources"));
        alert.setInformativeText(ns_string!(
            "Removing trust stops affected workers. Folder entries include subfolders."
        ));
        alert.addButtonWithTitle(ns_string!("Close"));
        if entries.is_empty() {
            alert.setInformativeText(ns_string!("No trusted locations"));
            alert.runModal();
            return Ok(());
        }
        let choices = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            NSRect::new(NSPoint::ZERO, NSSize::new(DETAILS_SIZE.width, 32.0)),
            false,
        );
        for entry in &entries {
            choices.addItemWithTitle(&NSString::from_str(&format!(
                "{}: {}",
                match entry.kind {
                    Kind::File => "File",
                    Kind::Folder => "Folder (including subfolders)",
                },
                entry.path.display()
            )));
        }
        alert.setAccessoryView(Some(&choices));
        alert
            .addButtonWithTitle(ns_string!("Remove"))
            .setHasDestructiveAction(true);
        if alert.runModal() != NSAlertSecondButtonReturn {
            return Ok(());
        }
        shrimply_trust_core::remove(
            &entries[usize::try_from(choices.indexOfSelectedItem()).expect("trust row selected")],
        )?;
    }
}
