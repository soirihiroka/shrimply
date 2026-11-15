use crate::ActionButton;
use objc2::{MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSControlSize, NSModalResponseCancel, NSModalResponseOK,
    NSPopUpButton, NSTextAlignment, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

pub const WIDTH: f64 = 590.0;
pub const ROW_HEIGHT: f64 = 30.0;
const LABEL_WIDTH: f64 = 185.0;
const CONTROL_X: f64 = 200.0;
const CONTROL_WIDTH: f64 = 350.0;
const BUTTON_WIDTH: f64 = 140.0;
const BUTTON_HEIGHT: f64 = 32.0;
const BUTTON_BOTTOM: f64 = 16.0;
const BUTTON_RIGHT: f64 = 30.0;
const BUTTON_GAP: f64 = 10.0;

pub fn new(title: &str, height: f64, mtm: MainThreadMarker) -> Retained<NSWindow> {
    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, height));
    let sheet = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { sheet.setReleasedWhenClosed(false) };
    sheet.setTitle(&NSString::from_str(title));
    let content = NSView::initWithFrame(NSView::alloc(mtm), frame);
    let confirm_x = WIDTH - BUTTON_RIGHT - BUTTON_WIDTH;
    for (title, response, key, x) in [
        (
            "Cancel",
            NSModalResponseCancel,
            "\u{1b}",
            confirm_x - BUTTON_GAP - BUTTON_WIDTH,
        ),
        ("Choose File", NSModalResponseOK, "\r", confirm_x),
    ] {
        let button = ActionButton::new(
            title,
            move || {
                NSApplication::sharedApplication(mtm).stopModalWithCode(response);
            },
            mtm,
        );
        button.view().setFrame(NSRect::new(
            NSPoint::new(x, BUTTON_BOTTOM),
            NSSize::new(BUTTON_WIDTH, BUTTON_HEIGHT),
        ));
        button.view().setControlSize(NSControlSize::Large);
        button.view().setKeyEquivalent(&NSString::from_str(key));
        content.addSubview(button.view());
    }
    sheet.setContentView(Some(&content));
    sheet
}

pub fn run(parent: &NSWindow, sheet: &NSWindow) -> bool {
    parent.beginSheet_completionHandler(sheet, None);
    let response = NSApplication::sharedApplication(parent.mtm()).runModalForWindow(sheet);
    parent.endSheet(sheet);
    response == NSModalResponseOK
}

pub fn popup_row(
    title: &str,
    choices: &[&str],
    row: usize,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Retained<NSPopUpButton>) {
    let row_view = row_view(row, mtm);
    row_view.addSubview(&label(title, mtm));
    let control = crate::menu::popup(
        NSRect::new(
            NSPoint::new(CONTROL_X, 0.0),
            NSSize::new(CONTROL_WIDTH, ROW_HEIGHT),
        ),
        false,
        mtm,
    );
    add_choices(&control, choices);
    row_view.addSubview(&control);
    (row_view, control)
}

pub fn text_row(
    title: &str,
    value: &str,
    row: usize,
    mtm: MainThreadMarker,
) -> (Retained<NSView>, Retained<NSTextField>) {
    let row_view = row_view(row, mtm);
    row_view.addSubview(&label(title, mtm));
    let control = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(
            NSPoint::new(CONTROL_X, 2.0),
            NSSize::new(CONTROL_WIDTH, ROW_HEIGHT - 4.0),
        ),
    );
    control.setStringValue(&NSString::from_str(value));
    row_view.addSubview(&control);
    (row_view, control)
}

fn row_view(row: usize, mtm: MainThreadMarker) -> Retained<NSView> {
    NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(
            NSPoint::new(20.0, 60.0 + row as f64 * ROW_HEIGHT),
            NSSize::new(WIDTH - 40.0, ROW_HEIGHT),
        ),
    )
}

fn label(title: &str, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setAlignment(NSTextAlignment::Right);
    label.setFrame(NSRect::new(
        NSPoint::new(0.0, 5.0),
        NSSize::new(LABEL_WIDTH, 20.0),
    ));
    label
}

pub fn add_choices(control: &NSPopUpButton, choices: &[&str]) {
    for choice in choices {
        control.addItemWithTitle(&NSString::from_str(choice));
    }
}
