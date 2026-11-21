use objc2::{MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSBox, NSBoxType, NSButtonType,
    NSMenu, NSScrollView, NSUserInterfaceItemIdentification, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use shrimply_components_appkit::ActionButton;

pub(super) const IDENTIFIER: &str = "shrimply-menu-sheet";
const INSET: f64 = 12.0;
const ROW_HEIGHT: f64 = 44.0;
const SEPARATOR_HEIGHT: f64 = 12.0;

/// Render the same native menu actions and custom controls in a scrollable sheet.
/// No timeline selection or action semantics are interpreted by this adapter.
pub(super) fn present(view: &NSView, menu: &Retained<NSMenu>, point: glam::Vec2, touch: bool) {
    if !shrimply_components_core::layout::menu_uses_bottom_sheet(if touch {
        "touch"
    } else {
        "pointer"
    }) {
        menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            NSPoint::new(point.x.into(), point.y.into()),
            Some(view),
        );
        return;
    }
    let parent = view.window().expect("native menu requires a window");
    let mtm = parent.mtm();
    let parent_bounds = parent
        .contentView()
        .expect("menu parent has content")
        .bounds();
    let items = menu.itemArray();
    let heights = items
        .iter()
        .map(|item| {
            if item.isSeparatorItem() {
                SEPARATOR_HEIGHT
            } else {
                item.view()
                    .map_or(ROW_HEIGHT, |view| view.frame().size.height.max(ROW_HEIGHT))
            }
        })
        .collect::<Vec<_>>();
    let document_height = heights.iter().sum::<f64>() + ROW_HEIGHT + INSET * 2.0;
    let size = NSSize::new(
        parent_bounds.size.width,
        document_height.min(parent_bounds.size.height),
    );
    let sheet = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::ZERO, size),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { sheet.setReleasedWhenClosed(false) };
    sheet.setIdentifier(Some(&NSString::from_str(IDENTIFIER)));
    let document = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::ZERO, NSSize::new(size.width, document_height)),
    );
    document.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    let mut top = document_height - INSET;
    for (index, (item, height)) in items.iter().zip(heights).enumerate() {
        top -= height;
        let frame = NSRect::new(
            NSPoint::new(INSET, top),
            NSSize::new(size.width - INSET * 2.0, height),
        );
        if item.isSeparatorItem() {
            let separator = NSBox::initWithFrame(NSBox::alloc(mtm), frame);
            separator.setBoxType(NSBoxType::Separator);
            separator.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            document.addSubview(&separator);
        } else if let Some(control) = item.view() {
            for child in control.subviews() {
                child.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            }
            control.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            control.setFrame(frame);
            document.addSubview(&control);
        } else {
            let menu = menu.clone();
            let action = ActionButton::new(
                &item.title().to_string(),
                move || {
                    menu.performActionForItemAtIndex(index.try_into().expect("native menu index"));
                    NSApplication::sharedApplication(mtm).stopModal();
                },
                mtm,
            );
            action.view().setFrame(frame);
            action.view().setEnabled(item.isEnabled());
            action.view().setImage(item.image().as_deref());
            if item.representedObject().is_some() {
                action.view().setButtonType(NSButtonType::Switch);
                action.view().setState(item.state());
            }
            action
                .view()
                .setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            document.addSubview(action.view());
        }
    }
    let cancel = ActionButton::new(
        shrimply_i18n::text("Cancel").as_ref(),
        move || {
            NSApplication::sharedApplication(mtm).stopModal();
        },
        mtm,
    );
    cancel
        .view()
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    cancel.view().setFrame(NSRect::new(
        NSPoint::new(INSET, INSET),
        NSSize::new(size.width - INSET * 2.0, ROW_HEIGHT),
    ));
    cancel
        .view()
        .setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    document.addSubview(cancel.view());
    let scroll =
        NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
    scroll.setHasVerticalScroller(true);
    scroll.setAutohidesScrollers(true);
    scroll.setDocumentView(Some(&document));
    sheet.setContentView(Some(&scroll));
    parent.beginSheet_completionHandler(&sheet, None);
    scroll.contentView().scrollToPoint(NSPoint::new(
        0.0,
        document_height - scroll.contentView().bounds().size.height,
    ));
    scroll.reflectScrolledClipView(&scroll.contentView());
    NSApplication::sharedApplication(mtm).runModalForWindow(&sheet);
    parent.endSheet(&sheet);
    sheet.orderOut(None);
}
