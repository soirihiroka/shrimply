use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::scene::pointer) fn begin_pointer_action(
    pos: Vec2,
    project: &Project,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    runtime: &mut Scene,
    width: f64,
    timeline_width: f64,
    height: f64,
    track_content_height: f64,
    duration_seconds: f64,
) {
    let x = pos.x as f64;
    let y = pos.y as f64;
    let frame_step_seconds = frame_step_seconds(project);
    let selected_items = selected_timeline_items(selection_state);
    runtime.view.clamp(
        duration_seconds,
        timeline_width,
        min_seconds_per_pixel(frame_step_seconds),
        track_content_height,
        height,
    );
    runtime.view.drag_start_x = x;
    runtime.view.drag_start_y = y;
    runtime.view.drag_start_scroll_seconds = runtime.view.scroll_seconds;
    runtime.view.drag_start_scroll_y = runtime.view.scroll_y;
    runtime.view.drag_start_seconds_per_pixel = runtime.view.seconds_per_pixel;
    runtime.view.drag_moved = false;
    runtime.view.selection = None;
    runtime.dragged_group = None;
    runtime.folded_drag = None;
    runtime.resize_drag = None;
    runtime.clip_transition_drag = None;
    runtime.horizontal_scrollbar.cancel_scroll();
    runtime.vertical_scrollbar.cancel_scroll();
    runtime.pressed_track_selection = None;

    let (vertical_drag_mode, vertical_target_handled) = begin_vertical_slider_action(
        &mut runtime.view,
        &mut runtime.vertical_scrollbar,
        x,
        y,
        width,
        height,
        track_content_height,
    );
    let (horizontal_drag_mode, horizontal_target_handled) =
        if matches!(vertical_drag_mode, DragMode::VerticalSliderMove) || vertical_target_handled {
            (DragMode::None, false)
        } else {
            begin_slider_action(
                &mut runtime.view,
                &mut runtime.horizontal_scrollbar,
                x,
                y,
                timeline_width,
                height,
                duration_seconds,
            )
        };
    runtime.view.drag_mode = if matches!(vertical_drag_mode, DragMode::VerticalSliderMove) {
        vertical_drag_mode
    } else if vertical_target_handled {
        DragMode::None
    } else if !matches!(horizontal_drag_mode, DragMode::None) || horizontal_target_handled {
        horizontal_drag_mode
    } else if runtime.suppress_double_click_selection
        && let Some(path) = crate::folded_sequence::hit_folded_item(project, runtime.view, x, y)
    {
        runtime.pending_sequence_toggle = Some(path);
        DragMode::None
    } else if let Some(id) = track_button_at(project, runtime.view, x, y) {
        let response = runtime
            .track_buttons
            .entry(id)
            .or_default()
            .event(shrimply_skia_adw_core::button::Event::Pressed);
        runtime.track_controls_animating |= response.animating;
        runtime.pressed_track_button = response.handled.then_some(id);
        DragMode::None
    } else if let Some((key, action)) = track_label_action_at(project, runtime.view, x, y) {
        debug_assert_eq!(action, TrackLabelAction::Select);
        runtime.pressed_track_selection = Some(key);
        DragMode::None
    } else if x < timeline_x() || x > timeline_x() + timeline_width {
        DragMode::None
    } else if y < RULER_HEIGHT && x >= timeline_x() && x <= timeline_x() + timeline_width {
        let position = Time::from_seconds_f64(
            x_to_time(
                x,
                runtime.view.scroll_seconds,
                runtime.view.seconds_per_pixel,
            )
            .max(0.0),
        );
        let position = runtime.snap_repository.snap(position).unwrap_or(position);
        player_state::set_scrubbing(player_state, true);
        runtime.pending_seek = Some(position);
        DragMode::Seek
    } else if let Some(gesture) = crate::transitions::Gesture::begin(
        project,
        selection_state,
        runtime.view,
        glam::DVec2::new(x, y),
    ) {
        runtime.clip_transition_drag = gesture.clip;
        runtime.transition_drag = gesture.item;
        if runtime.clip_transition_drag.is_some() || runtime.transition_drag.is_some() {
            runtime.pending_pause_playback = true;
            DragMode::Transition
        } else {
            DragMode::None
        }
    } else if let Some(hit) =
        crate::folded_sequence::hit_projected_item(project, runtime.view, x, y)
    {
        let (item_x, item_width) =
            crate::drawing::item_rect(hit.start, hit.end, timeline_x(), runtime.view);
        let drag_kind = if x <= item_x + ITEM_RESIZE_HANDLE_WIDTH {
            crate::folded_sequence::FoldedDragKind::ResizeStart
        } else if x >= item_x + item_width - ITEM_RESIZE_HANDLE_WIDTH {
            crate::folded_sequence::FoldedDragKind::ResizeEnd
        } else {
            crate::folded_sequence::FoldedDragKind::Move
        };
        let context = SequenceTimeline::for_item(project, &hit.key)
            .expect("projected item must have a valid operation scope");
        let selected = select_item_in_context(
            &context,
            project,
            selection_state,
            hit.key.clone(),
            runtime.modifiers.ctrl,
            runtime.modifiers.shift,
        );
        if selected {
            if runtime.cut_enabled {
                let selected = selection_state::selected_item_addresses(selection_state, project);
                runtime.cut_preview = cut_time_for_address(
                    project,
                    runtime.view,
                    &hit.key,
                    x,
                    &runtime.snap_repository,
                )
                .map(|time| timeline_cut(project, &selected, hit.key, time));
                if runtime.cut_preview.is_some() {
                    DragMode::Cut
                } else {
                    DragMode::None
                }
            } else {
                runtime.folded_drag = crate::folded_sequence::begin_drag(
                    project,
                    hit,
                    drag_kind,
                    x_to_time(
                        x,
                        runtime.view.scroll_seconds,
                        runtime.view.seconds_per_pixel,
                    ),
                    &selection_state::selected_item_addresses(selection_state, project),
                );
                runtime.pending_pause_playback = true;
                if drag_kind == crate::folded_sequence::FoldedDragKind::Move {
                    DragMode::Item
                } else {
                    DragMode::ResizeItem
                }
            }
        } else {
            DragMode::None
        }
    } else if let Some((hit, edge)) =
        hit_resize_handle_at(project, runtime.view, x, y, ITEM_RESIZE_HANDLE_WIDTH)
    {
        let resize_selection = selected_items.clone();
        if !resize_selection.contains(&hit) {
            set_selection(project, selection_state, vec![hit], Some(hit), true);
        }
        runtime.resize_drag = resize_drag_for_hit(
            project,
            &resize_selection,
            hit,
            edge,
            runtime.drag_collision_mode,
        );
        if runtime.resize_drag.is_some() {
            runtime.pending_pause_playback = true;
            DragMode::ResizeItem
        } else {
            DragMode::None
        }
    } else if runtime.cut_enabled {
        let hit_started = Instant::now();
        let hit = hit_item_at(project, runtime.view, x, y);
        let hit_us = hit_started.elapsed().as_micros();
        if let Some(hit) = hit {
            let address = selection_state::item_address(project, hit)
                .expect("hit-tested root item must have an address");
            let cut_started = Instant::now();
            let time =
                cut_time_for_address(project, runtime.view, &address, x, &runtime.snap_repository);
            let cut_time_us = cut_started.elapsed().as_micros();
            tracing::debug!(
                "timeline cut begin hit={}#{}:{} x={:.1} y={:.1} time={} hit_us={} cut_time_us={}",
                hit.kind.label(),
                hit.track_index,
                hit.item_index,
                x,
                y,
                time.map(|time| format!("{:.3}", time.as_secs_f64()))
                    .unwrap_or_else(|| "none".to_string()),
                hit_us,
                cut_time_us
            );
            if let Some(time) = time {
                let selected = selection_state::selected_item_addresses(selection_state, project);
                runtime.cut_preview = Some(timeline_cut(project, &selected, address, time));
                DragMode::Cut
            } else {
                runtime.cut_preview = None;
                DragMode::None
            }
        } else {
            tracing::debug!("timeline cut begin no-hit x={x:.1} y={y:.1} hit_us={hit_us}");
            runtime.cut_preview = None;
            if !runtime.modifiers.ctrl
                && !runtime.modifiers.shift
                && let Some(gap) = hit_gap_at(project, runtime.view, x, y)
            {
                selection_state::set_selected_gap(selection_state, Some(gap));
            }
            DragMode::None
        }
    } else if let Some(hit) = hit_item_at(project, runtime.view, x, y) {
        let address = selection_state::item_address(project, hit)
            .expect("hit-tested root item must have an address");
        let selected = select_item_in_context(
            &SequenceTimeline::root(),
            project,
            selection_state,
            address,
            runtime.modifiers.ctrl,
            runtime.modifiers.shift,
        );
        if selected {
            let selected_items = selected_timeline_items(selection_state);
            if let Some(group) = dragged_group_for_hit(
                project,
                &selected_items,
                hit,
                runtime.view,
                x,
                runtime.drag_collision_mode,
            ) {
                runtime.pending_pause_playback = true;
                runtime.dragged_group = Some(group);
                DragMode::Item
            } else {
                DragMode::None
            }
        } else {
            DragMode::None
        }
    } else {
        if runtime.suppress_double_click_selection {
            DragMode::None
        } else {
            let time = Time::from_seconds_f64(
                x_to_time(
                    x,
                    runtime.view.scroll_seconds,
                    runtime.view.seconds_per_pixel,
                )
                .max(0.0),
            );
            let time = runtime.snap_repository.snap(time).unwrap_or(time);
            let y = content_y(runtime.view, y);
            shrimply_support::crash::set_context(format!(
                "timeline rectangle selection begin seconds={:.6} y={y:.1} add_to_selection={} ignore_grouping={}",
                time.as_secs_f64(),
                runtime.modifiers.ctrl,
                runtime.modifiers.shift
            ));
            runtime.view.selection = Some(TimelineSelection {
                start: time,
                end: time,
                start_y: y,
                end_y: y,
                add_to_selection: runtime.modifiers.ctrl,
                ignore_grouping: runtime.modifiers.shift,
            });
            DragMode::Select
        }
    };
}
