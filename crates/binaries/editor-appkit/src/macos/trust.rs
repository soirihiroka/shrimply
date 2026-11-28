use super::*;
use objc2_app_kit::{NSScrollView, NSTextField};
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
