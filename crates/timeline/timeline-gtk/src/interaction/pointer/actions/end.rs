use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::interaction::pointer) fn end_pointer_action(
    pos: Vec2,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &mut TimelineRuntime,
    _width: f64,
    _timeline_width: f64,
    _height: f64,
    _track_content_height: f64,
) {
    let x = pos.x as f64;
    let y = pos.y as f64;
    if let Some(id) = runtime.pressed_track_button.take() {
        let response = runtime
            .track_buttons
            .entry(id)
            .or_default()
            .event(shrimply_skia_adw_core::button::Event::Released);
        runtime.track_controls_animating |= response.animating;
        if response.clicked {
            activate_track_button(runtime, selection_state, id);
        }
    }
    if let Some(pressed_key) = runtime.pressed_track_selection.take()
        && !runtime.view.drag_moved
    {
        let released_action = {
            let project = project.borrow();
            track_label_action_at(&project, runtime.view, x, y)
        };
        if released_action == Some((pressed_key, TrackLabelAction::Select)) {
            let ctrl = runtime.modifiers.ctrl;
            let shift = runtime.modifiers.shift;
            select_track(selection_state, pressed_key, ctrl, shift);
        }
    }
    if matches!(
        runtime.view.drag_mode,
        DragMode::Item | DragMode::ResizeItem
    ) && dragging::finish(
        runtime,
        project,
        player_state,
        selection_state,
        glam::DVec2::new(x, y),
    ) {
        runtime.view.drag_mode = DragMode::None;
    }
    match runtime.view.drag_mode {
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
            if let Some(mut selection) = runtime.view.selection {
                selection.end = end;
                selection.end_y = end_y;
                shrimply_support::crash::set_context(format!(
                    "timeline rectangle selection end moved={} range={:.6}..{:.6} y={:.1}..{:.1} add_to_selection={} ignore_grouping={}",
                    runtime.view.drag_moved,
                    selection.start.as_secs_f64(),
                    selection.end.as_secs_f64(),
                    selection.start_y,
                    selection.end_y,
                    selection.add_to_selection,
                    selection.ignore_grouping
                ));
                if runtime.view.drag_moved {
                    let project = project.borrow();
                    let row = ((selection.start_y - RULER_HEIGHT) / TRACK_HEIGHT).floor() as usize;
                    match crate::items::track_rows(&project).get(row) {
                        Some(track_row) if track_row.root_key.is_none() => {
                            let context = SequenceTimeline::for_track(&project, &track_row.address)
                                .expect("timeline row must have a valid operation scope");
                            commit_rectangle_selection(
                                &context,
                                &project,
                                selection_state,
                                selection,
                            );
                        }
                        Some(_) | None => {
                            commit_rectangle_selection(
                                &SequenceTimeline::root(),
                                &project,
                                selection_state,
                                selection,
                            );
                        }
                    }
                } else if !selection.add_to_selection {
                    let project = project.borrow();
                    if !selection.ignore_grouping
                        && let Some(gap) = hit_gap_at(&project, runtime.view, x, y)
                    {
                        selection_state::set_selected_gap(selection_state, Some(gap));
                    } else {
                        set_selection(&project, selection_state, Vec::new(), None, true);
                    }
                }
            }
        }
        DragMode::Item | DragMode::ResizeItem => {}
        DragMode::Transition => {
            let mut gesture = shrimply_timeline_core::transitions::Gesture {
                clip: runtime.clip_transition_drag.take(),
                item: runtime.transition_drag.take(),
            };
            if runtime.view.drag_moved {
                gesture.update(
                    &project.borrow(),
                    selection_state,
                    runtime.view,
                    x,
                    &runtime.snap_repository,
                );
            }
            let mut project_state = project.borrow_mut();
            if let Some(applied) = gesture.finish(&mut project_state, runtime.view.drag_moved) {
                project_state.normalize_clip_transitions();
                crate::project::commit_edit(&project_state, applied.message);
                drop(project_state);
                if let Some(side) = applied.focus {
                    selection_state::set_focused_transition(selection_state, side);
                } else {
                    selection_state::clear_focused_transition(selection_state);
                }
                let audio = applied.kind == crate::project::ItemKind::Audio;
                player_state::refresh_project(
                    player_state,
                    ProjectChange {
                        audio,
                        audio_waveforms: audio && applied.rolling,
                        video: applied.kind == crate::project::ItemKind::Video,
                        inspector: true,
                        ..ProjectChange::default()
                    },
                );
            }
        }
        DragMode::Cut => {
            tracing::debug!(
                "timeline cut end moved={} has_preview={}",
                runtime.view.drag_moved,
                runtime.cut_preview.is_some()
            );
            if !runtime.view.drag_moved
                && let Some(cut) = runtime.cut_preview.take()
            {
                let total_started = Instant::now();
                let borrow_started = Instant::now();
                let mut project_state = project.borrow_mut();
                let borrow_us = borrow_started.elapsed().as_micros();
                let split_started = Instant::now();
                let context = SequenceTimeline::for_item(&project_state, &cut.key)
                    .expect("cut item must have a valid operation scope");
                let (changed, _) =
                    split_item_addresses(&context, &mut project_state, &cut.keys, cut.time);
                let split_us = split_started.elapsed().as_micros();
                if !changed.is_empty() {
                    project_state.normalize_clip_transitions();
                    let focused_item = changed
                        .iter()
                        .find(|item| item.item_id() == cut.key.item_id())
                        .cloned();
                    let duration_started = Instant::now();
                    let duration = project_state.duration();
                    let duration_us = duration_started.elapsed().as_micros();
                    let change_started = Instant::now();
                    let mut change = ProjectChange {
                        duration: Some(duration),
                        ..ProjectChange::default()
                    };
                    for key in &cut.keys {
                        match key.kind() {
                            crate::project::ItemKind::Caption => change.captions = true,
                            crate::project::ItemKind::Video => change.video = true,
                            crate::project::ItemKind::Audio => {
                                change.audio = true;
                                change.audio_waveforms = true;
                            }
                        }
                    }
                    let change_us = change_started.elapsed().as_micros();
                    let commit_started = Instant::now();
                    crate::project::commit_edit(&project_state, "split-timeline-item");
                    let commit_us = commit_started.elapsed().as_micros();
                    drop(project_state);
                    let project = project.borrow();
                    let selection_started = Instant::now();
                    selection_state::set_selected_item_addresses(
                        selection_state,
                        &project,
                        changed,
                        focused_item,
                    );
                    let selection_us = selection_started.elapsed().as_micros();
                    let refresh_started = Instant::now();
                    player_state::refresh_project(player_state, change);
                    let refresh_us = refresh_started.elapsed().as_micros();
                    tracing::debug!(
                        "timeline cut apply key={:?}:{} preview_keys={} borrow_us={} split_us={} duration_us={} change_us={} commit_us={} selection_us={} refresh_us={} total_us={}",
                        cut.key.kind(),
                        cut.key.item_id(),
                        cut.keys.len(),
                        borrow_us,
                        split_us,
                        duration_us,
                        change_us,
                        commit_us,
                        selection_us,
                        refresh_us,
                        total_started.elapsed().as_micros()
                    );
                } else {
                    drop(project_state);
                    tracing::debug!(
                        "timeline cut apply key={:?}:{} preview_keys={} changed=0 borrow_us={} split_us={} total_us={}",
                        cut.key.kind(),
                        cut.key.item_id(),
                        cut.keys.len(),
                        borrow_us,
                        split_us,
                        total_started.elapsed().as_micros()
                    );
                }
            }
        }
        DragMode::None
        | DragMode::Seek
        | DragMode::MiddlePan
        | DragMode::SliderMove
        | DragMode::VerticalSliderMove => {}
    }

    runtime.dragged_group = None;
    runtime.resize_drag = None;
    runtime.transition_drag = None;
    runtime.clip_transition_drag = None;
    runtime.cut_preview = None;
    runtime.view.selection = None;
    runtime.horizontal_scrollbar.end_drag();
    runtime.vertical_scrollbar.end_drag();
    runtime.view.drag_mode = DragMode::None;
    runtime.view.drag_moved = false;
}

use shrimply_timeline_core::selection::commit_rectangle_selection;
