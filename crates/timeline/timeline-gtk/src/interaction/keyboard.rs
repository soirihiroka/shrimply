use super::*;
use shrimply_gtk_components::ui::I18nAlertDialogExt;
use shrimply_timeline_core::scene::KeyAction;

pub(super) fn add_controller(
    area: &gtk::GLArea,
    project: Rc<RefCell<Project>>,
    player_state: SharedPlayerState,
    selection_state: SharedSelectionState,
    runtime: Rc<RefCell<TimelineRuntime>>,
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
        // GTK owns stabilization's native service callback when pasting properties.
        if matches!(action, KeyAction::ReplaceProperties) {
            replace_selected_item_properties(
                &area_for_key,
                &project,
                &player_state,
                &selection_state,
                &runtime,
            );
            return glib::Propagation::Stop;
        }
        let result = runtime.borrow_mut().scene.key_action(action);
        match result {
            Ok(Some(crate::ContextMenuRequest::SetTimelineClipboardMarker)) => {
                area_for_key
                    .display()
                    .clipboard()
                    .set_text(crate::clipboard::TIMELINE_MARKER);
            }
            Ok(Some(crate::ContextMenuRequest::PasteFromClipboard)) => {
                crate::clipboard::paste(
                    &area_for_key,
                    &project,
                    &player_state,
                    &selection_state,
                    &runtime,
                );
            }
            Ok(Some(crate::ContextMenuRequest::DeleteTracks { clip_count })) => {
                let deletion = runtime.borrow().scene.track_deletion();
                let dialog = adw::AlertDialog::new(
                    Some("Delete Tracks?"),
                    Some(&format!(
                        "{clip_count} clips are about to be deleted, are you sure?"
                    )),
                );
                dialog.add_responses_i18n(&[("cancel", "Cancel"), ("delete", "Delete")]);
                dialog.set_close_response("cancel");
                dialog.set_default_response(Some("delete"));
                dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
                let runtime = Rc::downgrade(&runtime);
                let area = area_for_key.clone();
                dialog.choose(
                    Some(&area_for_key),
                    None::<&gio::Cancellable>,
                    move |response| {
                        if response == "delete"
                            && let Some(runtime) = runtime.upgrade()
                        {
                            let result = runtime
                                .borrow_mut()
                                .scene
                                .confirm_delete_selected_tracks(deletion);
                            if let Err(error) = result {
                                show_error_dialog(&area, "Timeline edit failed", &error);
                            }
                            area.queue_render();
                        }
                    },
                );
            }
            Ok(Some(request)) => {
                unreachable!("timeline keyboard returned a non-keyboard request: {request:?}")
            }
            Ok(None) => {}
            Err(error) => show_error_dialog(&area_for_key, "Timeline edit failed", &error),
        }
        area_for_key.queue_render();
        glib::Propagation::Stop
    });
    area.add_controller(controller);
}
