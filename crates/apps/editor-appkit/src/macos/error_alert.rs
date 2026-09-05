use objc2_app_kit::{NSFont, NSTextView};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString, ns_string};

const DETAILS_WIDTH: f64 = 620.0;
const DETAILS_HEIGHT: f64 = 300.0;
const DETAILS_MINIMUM_LENGTH: usize = 240;

pub fn show(mtm: MainThreadMarker, error: &str) {
    let alert = objc2_app_kit::NSAlert::new(mtm);
    alert.setMessageText(ns_string!("Shrimply"));
    if error.len() < DETAILS_MINIMUM_LENGTH && error.lines().count() < 5 {
        alert.setInformativeText(&NSString::from_str(error));
        alert.runModal();
        return;
    }
    let summary = error
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("An operation failed");
    alert.setInformativeText(&NSString::from_str(summary));

    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(DETAILS_WIDTH, DETAILS_HEIGHT));
    let scroll = NSTextView::scrollableTextView(mtm);
    scroll.setFrame(frame);
    let details = scroll
        .documentView()
        .and_then(|view| view.downcast::<NSTextView>().ok())
        .expect("AppKit's scrollable text view must contain an NSTextView");
    details.setEditable(false);
    details.setSelectable(true);
    details.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(
        12.0,
        unsafe { objc2_app_kit::NSFontWeightRegular },
    )));
    details.setString(&NSString::from_str(error));
    alert.setAccessoryView(Some(&scroll));
    alert.runModal();
}
