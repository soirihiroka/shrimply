use super::*;
use shrimply_timeline_skia::scene::KeyAction;

pub(super) fn add_controller(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
    preferences: preferences_store::SharedPreferences,
) {
    let controller = gtk::EventControllerKey::new();
    let area_for_key = area.clone();
    controller.connect_key_pressed(move |_, key, _, state| {
        if key == gdk::Key::Escape {
            runtime.borrow_mut().scene.pointer_cancelled();
            area_for_key.queue_render();
            return glib::Propagation::Stop;
        }
        if state.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK) {
            return glib::Propagation::Proceed;
        }
        let character = match key {
            gdk::Key::BackSpace => Some('\u{8}'),
            gdk::Key::Delete => Some('\u{7f}'),
            _ => key.to_unicode(),
        };
        let Some(action) = character.and_then(|key| {
            KeyAction::from_key(
                key,
                state.contains(gdk::ModifierType::CONTROL_MASK),
                state.contains(gdk::ModifierType::SHIFT_MASK),
            )
        }) else {
            return glib::Propagation::Proceed;
        };
        let result = runtime.borrow_mut().scene.key_action(action);
        context_actions::handle_action_result(
            &area_for_key,
            &project,
            &player_state,
            &selection_state,
            &runtime,
            &preferences,
            result,
        );
        glib::Propagation::Stop
    });
    area.add_controller(controller);
}
