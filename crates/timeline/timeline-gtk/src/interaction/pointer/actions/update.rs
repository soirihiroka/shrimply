use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::interaction::pointer) fn update_pointer_action(
    pos: Vec2,
    project: &Project,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &mut TimelineRuntime,
    width: f64,
    timeline_width: f64,
    height: f64,
    track_content_height: f64,
    duration_seconds: f64,
) {
    let x = pos.x as f64;
    let y = pos.y as f64;
    let offset_x = x - runtime.view.drag_start_x;
    let offset_y = y - runtime.view.drag_start_y;
    if offset_x.abs().max(offset_y.abs()) > CLICK_DRAG_TOLERANCE {
        runtime.view.drag_moved = true;
    }

    let frame_step_seconds = frame_step_seconds(project);
    let drag_mode = runtime.view.drag_mode;
    match drag_mode {
        DragMode::None => {}
        DragMode::Seek => {
            let position = Time::from_seconds_f64(
                x_to_time(
                    x,
                    runtime.view.scroll_seconds,
                    runtime.view.seconds_per_pixel,
                )
                .max(0.0),
            );
            let position = runtime.snap_repository.snap(position).unwrap_or(position);
            crate::recording::stop_before_backward_seek(runtime, player_state, position);
            player_state::seek_time(player_state, position);
        }
        DragMode::Select => {
            let end = Time::from_seconds_f64(
                x_to_time(
                    x,
                    runtime.view.scroll_seconds,
                    runtime.view.seconds_per_pixel,
                )
                .max(0.0),
            );
            let end = runtime.snap_repository.snap(end).unwrap_or(end);
            let end_y = content_y(runtime.view, y);
            if let Some(selection) = runtime.view.selection.as_mut() {
                selection.end = end;
                selection.end_y = end_y;
            }
            runtime.view.clamp(
                duration_seconds,
                timeline_width,
                min_seconds_per_pixel(frame_step_seconds),
                track_content_height,
                height,
            );
        }
        DragMode::Item => {
            dragging::update(runtime, project, glam::DVec2::new(x, y));
            runtime.view.clamp(
                duration_seconds,
                timeline_width,
                min_seconds_per_pixel(frame_step_seconds),
                track_content_height,
                height,
            );
        }
        DragMode::ResizeItem => {
            dragging::update(runtime, project, glam::DVec2::new(x, y));
        }
        DragMode::Transition => {
            let mut gesture = shrimply_timeline_core::transitions::Gesture {
                clip: runtime.clip_transition_drag.take(),
                item: runtime.transition_drag.take(),
            };
            gesture.update(
                project,
                selection_state,
                runtime.view,
                x,
                &runtime.snap_repository,
            );
            runtime.clip_transition_drag = gesture.clip;
            runtime.transition_drag = gesture.item;
        }
        DragMode::Cut => {
            update_cut_preview(runtime, project, selection_state, x, y);
        }
        DragMode::MiddlePan => {}
        DragMode::SliderMove => {
            update_slider_drag(
                &mut runtime.view,
                &mut runtime.horizontal_scrollbar,
                x,
                timeline_width,
                height,
                track_content_height,
                duration_seconds,
                min_seconds_per_pixel(frame_step_seconds),
                drag_mode,
            );
        }
        DragMode::VerticalSliderMove => {
            update_vertical_slider_drag(
                &mut runtime.view,
                &mut runtime.vertical_scrollbar,
                y,
                width,
                height,
                track_content_height,
            );
        }
    }
}
