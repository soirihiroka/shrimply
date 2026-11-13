use objc2_app_kit::{NSColorWell, NSControl, NSTextView, NSView};

pub(super) fn active(view: &NSView) -> bool {
    if view.window().is_none() || view.isHiddenOrHasHiddenAncestor() {
        return false;
    }
    if has_text_editor(view)
        || view
            .downcast_ref::<NSColorWell>()
            .is_some_and(NSColorWell::isActive)
    {
        return true;
    }
    view.subviews().iter().any(|child| active(&child))
}

fn has_text_editor(view: &NSView) -> bool {
    // A field editor is shared by the window and need not be a descendant of its
    // NSTextField. Walk the controls as well as checking standalone NSTextViews.
    view.downcast_ref::<NSControl>()
        .is_some_and(|control| control.currentEditor().is_some())
        || view.downcast_ref::<NSTextView>().is_some_and(|text| {
            text.isEditable()
                && text
                    .window()
                    .and_then(|window| window.firstResponder())
                    .and_then(|responder| responder.downcast::<NSTextView>().ok())
                    .is_some_and(|responder| std::ptr::eq(&*responder, text))
        })
}

pub(super) fn finish(view: &NSView) {
    if has_text_editor(view) {
        assert!(
            view.window()
                .expect("active editor must have a window")
                .makeFirstResponder(None),
            "inspector editor refused to finish before removal"
        );
    }
    if let Some(well) = view.downcast_ref::<NSColorWell>() {
        well.deactivate();
    }
    for child in view.subviews() {
        finish(&child);
    }
}

pub(super) fn deactivate_color_wells(view: &NSView) {
    if let Some(well) = view.downcast_ref::<NSColorWell>() {
        well.deactivate();
    }
    for child in view.subviews() {
        deactivate_color_wells(&child);
    }
}
