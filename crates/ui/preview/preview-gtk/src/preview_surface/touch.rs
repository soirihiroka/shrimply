use super::*;
use shrimply_components_skia::input::TouchPhase;

pub(super) fn attach(preview: &PreviewController) {
    let area = preview.widget();
    let project = preview.project.clone();
    let player = preview.player_state.clone();
    let selection = preview.selection_state.clone();
    let focus = preview.preview_focus.clone();
    let state = preview.surface.state.clone();
    shrimply_components_gtk::touch::connect(area, {
        let area = area.downgrade();
        let project = project.clone();
        let player = player.clone();
        let selection = selection.clone();
        let focus = focus.clone();
        let state = state.clone();
        move |change| {
            let Some(area) = area.upgrade() else {
                return;
            };
            if change.phase == TouchPhase::Begin {
                area.grab_focus();
                state.borrow().sanim_inspector.dismiss();
            }
            let surface = ProviderDispatch {
                area: &area,
                project: &project,
                player_state: &player,
                selection_state: &selection,
                preview_focus: &focus,
                state: &state,
            };
            let output = {
                let clock = surface.clock();
                let mut project = surface.project.borrow_mut();
                let selected =
                    selection_state::focused_video_address(surface.selection_state, &project);
                let mut state = surface.state.borrow_mut();
                let config = state.configuration(surface.area);
                state.sync_focus(surface.preview_focus);
                let result =
                    state
                        .surface
                        .touch(&mut project, selected.as_ref(), clock, config, &[change]);
                state.send_exclusion_update(clock.position);
                match result {
                    Ok(output) => output,
                    Err(error) => {
                        tracing::error!(%error, "Could not handle preview touch");
                        return;
                    }
                }
            };
            apply_response(&area, &player, &state, output.response);
            if let Some(redo) = output.history {
                player_state::change_history(
                    &project,
                    &player,
                    if redo {
                        crate::project::redo
                    } else {
                        crate::project::undo
                    },
                );
            }
            area.queue_render();
        }
    });
    let cancel = Rc::new(move |area: &gtk::GLArea| {
        cancel_provider(area, &project, &player, &state, true);
        area.set_cursor_from_name(None);
    });
    area.connect_unmap({
        let cancel = cancel.clone();
        move |area| cancel(area)
    });
    let focus_controller = gtk::EventControllerFocus::new();
    focus_controller.connect_leave(move |focus| {
        if let Some(area) = focus.widget().and_downcast::<gtk::GLArea>() {
            cancel(&area);
        }
    });
    area.add_controller(focus_controller);
}
