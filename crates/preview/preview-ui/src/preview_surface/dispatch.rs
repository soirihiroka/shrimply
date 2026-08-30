use super::*;

pub(super) fn attach_frame_scheduler(
    area: &gtk::GLArea,
    player_state: SharedPlayerState,
    controller: Rc<RefCell<PreviewControllerState>>,
) {
    area.add_tick_callback(move |area, _| {
        let mut controller_state = controller.borrow_mut();
        let live_base_pending =
            controller_state.live_base_pending && controller_state.live_base_in_flight.is_none();
        if live_base_pending {
            controller_state.live_base_pending = false;
        }
        let frame_pending = std::mem::take(&mut controller_state.frame_pending);
        if !frame_pending && !live_base_pending {
            return glib::ControlFlow::Continue;
        }
        drop(controller_state);
        if live_base_pending {
            player_state::refresh_project(
                &player_state,
                player_state::ProjectChange {
                    video: true,
                    live_preview: true,
                    ..Default::default()
                },
            );
            let revision = player_state::snapshot(&player_state).revision;
            let mut controller = controller.borrow_mut();
            controller.live_base_in_flight = Some(revision);
            if let Some(provider) = controller.provider.as_mut() {
                provider.project_revision = revision;
            }
        }
        if frame_pending {
            area.queue_render();
        }
        glib::ControlFlow::Continue
    });
}

pub(super) fn apply_response(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    response: PreviewResponse,
    commit_name: &'static str,
) {
    match response.cursor {
        CursorUpdate::Keep => {}
        CursorUpdate::Set(value) => area.set_cursor_from_name(Some(cursor_name(value))),
        CursorUpdate::Clear => area.set_cursor_from_name(None),
    }
    if response.edit.commits() {
        crate::project::commit_edit(&project.borrow(), commit_name);
    }
    if response.edit.refresh != PreviewRefresh::NONE {
        player_state::refresh_project(
            player_state,
            player_state::ProjectChange {
                video: response.edit.refresh.contains(PreviewRefresh::PREVIEW),
                live_preview: response.edit.is_live(),
                inspector: response.edit.refresh.contains(PreviewRefresh::INSPECTOR),
                ..Default::default()
            },
        );
    }
}

pub(super) fn caption_split_at_pointer(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    state: &Rc<RefCell<VideoSurfaceState>>,
    point: GlamVec2,
) -> Option<(ItemAddress, usize)> {
    let project = project.borrow();
    let address = selection_state::focused_item_address(selection_state, &project)?;
    let player = player_state::snapshot(player_state);
    let state = state.borrow();
    let preview_rect = geometry::display_content_rect(
        area.width().max(1),
        area.height().max(1),
        project.canvas_size.width,
        project.canvas_size.height,
        state.padding_px(),
        state.preview_zoom,
        state.preview_pan,
    );
    let text_byte = captions::split_at_position(
        &project,
        &address,
        player.position,
        preview_rect,
        state.caption_font_size,
        state.caption_background_color,
        state.caption_bottom_inset,
        point,
    )?;
    Some((address, text_byte))
}

pub(super) fn split_caption_at_pointer(
    area: &gtk::GLArea,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    state: &Rc<RefCell<VideoSurfaceState>>,
    point: GlamVec2,
) -> bool {
    let Some((address, text_byte)) =
        caption_split_at_pointer(area, project, player_state, selection_state, state, point)
    else {
        return false;
    };
    let cut = player_state::snapshot(player_state).position;
    let right = {
        let mut project = project.borrow_mut();
        let (_, right) =
            shrimply_timeline::edit::split_caption(&mut project, &address, cut, text_byte)
                .expect("previewed caption split must remain valid");
        crate::project::commit_edit(&project, "split-preview-caption");
        right
    };
    {
        let project = project.borrow();
        selection_state::set_selected_item_addresses(
            selection_state,
            &project,
            vec![right.clone()],
            Some(right),
        );
    }
    player_state::refresh_project(
        player_state,
        player_state::ProjectChange {
            captions: true,
            inspector: true,
            ..Default::default()
        },
    );
    area.queue_render();
    true
}
