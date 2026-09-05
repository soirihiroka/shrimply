use super::*;
pub(crate) use shrimply_timeline_core::scene::pointer::{content_y, set_timeline_selection};
pub(crate) use shrimply_timeline_core::selection::select_item_in_context;
pub(super) use shrimply_timeline_core::track_controls::select_track;
pub(super) fn push_modifiers(runtime: &Rc<RefCell<TimelineRuntime>>, state: gdk::ModifierType) {
    runtime
        .borrow_mut()
        .scene
        .event(shrimply_timeline_core::scene::Event::Modifiers(
            modifiers_from_state(state),
        ));
}
pub(super) fn modifiers_from_state(state: gdk::ModifierType) -> TimelineModifiers {
    TimelineModifiers {
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
    }
}
