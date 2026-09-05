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
            if let Some(mut drag) = runtime.clip_transition_drag.take() {
                if drag.original_duration.is_none()
                    && drag.handle.is_none()
                    && runtime.view.drag_moved
                {
                    let side = if x < runtime.view.drag_start_x {
                        TransitionSide::Outro
                    } else {
                        TransitionSide::Intro
                    };
                    let key = if side == TransitionSide::Outro {
                        drag.outgoing
                    } else {
                        drag.incoming
                    };
                    let context = SequenceTimeline::for_item(project, &key)
                        .expect("transition item must have a valid operation scope");
                    select_item_in_context(
                        &context,
                        project,
                        selection_state,
                        key.clone(),
                        false,
                        false,
                    );
                    selection_state::set_focused_transition(selection_state, side);
                    runtime.transition_drag = Some(TransitionDrag {
                        key,
                        side,
                        target_duration: Time::ZERO,
                        target_timeline_duration: Time::ZERO,
                        remove: false,
                    });
                } else {
                    update_clip_transition_drag(
                        &mut drag,
                        project,
                        runtime.view,
                        x,
                        &runtime.snap_repository,
                    );
                    runtime.clip_transition_drag = Some(drag);
                }
            }
            if let Some(drag) = runtime.transition_drag.as_mut() {
                update_transition_drag(drag, project, runtime.view, x, &runtime.snap_repository);
            }
        }
        DragMode::Cut => {
            let hit_started = Instant::now();
            let raw_hit = crate::folded_sequence::hit_projected_item(project, runtime.view, x, y)
                .map(|hit| hit.key)
                .or_else(|| {
                    hit_item_at(project, runtime.view, x, y)
                        .and_then(|key| selection_state::item_address(project, key))
                });
            let hit = raw_hit.as_ref().and_then(|key| {
                runtime
                    .cut_preview
                    .as_ref()
                    .is_none_or(|cut| cut.keys.contains(key))
                    .then_some(key.clone())
            });
            let hit_us = hit_started.elapsed().as_micros();
            if let Some(key) = hit {
                let cut_started = Instant::now();
                runtime.cut_preview =
                    cut_time_for_address(project, runtime.view, &key, x, &runtime.snap_repository)
                        .map(|time| {
                            timeline_cut(
                                project,
                                &selection_state::selected_item_addresses(selection_state, project),
                                key.clone(),
                                time,
                            )
                        });
                tracing::debug!(
                    "timeline cut update hit={:?}:{} x={:.1} y={:.1} preview={} hit_us={} cut_update_us={}",
                    key.kind(),
                    key.item_id(),
                    x,
                    y,
                    runtime.cut_preview.is_some(),
                    hit_us,
                    cut_started.elapsed().as_micros()
                );
            } else {
                if let Some(key) = raw_hit {
                    tracing::debug!(
                        "timeline cut update grouped-reject hit={:?}:{} x={:.1} y={:.1} hit_us={}",
                        key.kind(),
                        key.item_id(),
                        x,
                        y,
                        hit_us
                    );
                } else {
                    tracing::debug!("timeline cut update no-hit x={x:.1} y={y:.1} hit_us={hit_us}");
                }
                runtime.cut_preview = None;
            }
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

#[allow(clippy::too_many_arguments)]
pub(super) fn update_clip_transition_drag(
    drag: &mut ClipTransitionDrag,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    snap_repository: &SnapRepo,
) {
    if drag.center_resize {
        let track = drag.outgoing.track();
        let Some(timeline_cut) = project.sequence_time_to_timeline(&track, drag.cut) else {
            return;
        };
        let cut_x = timeline_x()
            + (timeline_cut.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
        let target = Time::from_seconds_f64(x_to_time(
            cut_x + x - view.drag_start_x,
            view.scroll_seconds,
            view.seconds_per_pixel,
        ));
        let target = snap_repository.snap(target).unwrap_or(target);
        let Some(target) = project
            .timeline_time_to_sequence(&track, target)
            .map(|target| target.snapped(project.frame_step()))
        else {
            return;
        };
        let Some((minimum, maximum)) =
            clip_transition_cut_range(drag, project, crate::geometry::frame_step(project))
        else {
            return;
        };
        drag.target_cut = target.max(minimum).min(maximum);
        return;
    }
    let Some(handle) = drag.handle else {
        return;
    };
    let target = Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));
    let target = snap_repository.snap(target).unwrap_or(target);
    let track = drag.outgoing.track();
    let Some(target) = project
        .timeline_time_to_sequence(&track, target)
        .map(|target| target.snapped(project.frame_step()))
    else {
        return;
    };
    let distance = match handle {
        ItemEdge::Start => drag.cut.saturating_sub(target),
        ItemEdge::End => target.saturating_sub(drag.cut),
    };
    let Some((left_start, left_end)) = project.item(&drag.outgoing).map(|item| item.times()) else {
        return;
    };
    let Some((right_start, right_end)) = project.item(&drag.incoming).map(|item| item.times())
    else {
        return;
    };
    let left_intro = transition_durations(project, &drag.outgoing).and_then(|(intro, _)| intro);
    let right_outro = transition_durations(project, &drag.incoming).and_then(|(_, outro)| outro);
    let maximum = crate::math::maximum_clip_transition_duration(
        left_end.saturating_sub(left_start),
        right_end.saturating_sub(right_start),
        left_intro,
        right_outro,
    );
    let duration = Time {
        seconds: distance.seconds * Fraction::from(2_u8),
    }
    .min(maximum);
    drag.target_duration = (duration > Time::ZERO).then_some(duration);
}

fn clip_transition_cut_range(
    drag: &ClipTransitionDrag,
    project: &Project,
    minimum_item_duration: Time,
) -> Option<(Time, Time)> {
    let duration = drag.original_duration?;
    let (left_start, _) = project.item(&drag.outgoing)?.times();
    let (_, right_end) = project.item(&drag.incoming)?.times();
    let left_intro = transition_durations(project, &drag.outgoing)?.0;
    let right_outro = transition_durations(project, &drag.incoming)?.1;
    let (previous_duration, next_duration) = surrounding_clip_transition_durations(drag, project)?;
    let left_minimum = crate::math::minimum_clip_transition_item_duration(duration, left_intro)
        .max(
            previous_duration
                .map(|duration| crate::math::minimum_clip_transition_item_duration(duration, None))
                .unwrap_or(minimum_item_duration),
        )
        .max(minimum_item_duration);
    let right_minimum = crate::math::minimum_clip_transition_item_duration(duration, right_outro)
        .max(
            next_duration
                .map(|duration| crate::math::minimum_clip_transition_item_duration(duration, None))
                .unwrap_or(minimum_item_duration),
        )
        .max(minimum_item_duration);
    let minimum = left_start.saturating_add(left_minimum);
    let maximum = right_end.saturating_sub(right_minimum);
    (minimum <= maximum).then_some((minimum, maximum))
}

fn surrounding_clip_transition_durations(
    drag: &ClipTransitionDrag,
    project: &Project,
) -> Option<(Option<Time>, Option<Time>)> {
    let track_address = drag.outgoing.track();
    match project.track(&track_address)? {
        crate::project::TrackRef::Video(track) => {
            let index = track
                .items
                .iter()
                .position(|item| item.id == drag.outgoing.item_id())?;
            let previous = index.checked_sub(1).and_then(|index| {
                track.items[index]
                    .transitions
                    .to_next
                    .as_ref()
                    .filter(|transition| transition.target_item_id == drag.outgoing.item_id())
                    .map(|transition| transition.duration)
            });
            let next = track
                .items
                .get(index + 1)?
                .transitions
                .to_next
                .as_ref()
                .filter(|transition| {
                    track
                        .items
                        .get(index + 2)
                        .is_some_and(|item| transition.target_item_id == item.id)
                })
                .map(|transition| transition.duration);
            Some((previous, next))
        }
        crate::project::TrackRef::Audio(track) => {
            let index = track
                .items
                .iter()
                .position(|item| item.id == drag.outgoing.item_id())?;
            let previous = index.checked_sub(1).and_then(|index| {
                track.items[index]
                    .transitions
                    .to_next
                    .as_ref()
                    .filter(|transition| transition.target_item_id == drag.outgoing.item_id())
                    .map(|transition| transition.duration)
            });
            let next = track
                .items
                .get(index + 1)?
                .transitions
                .to_next
                .as_ref()
                .filter(|transition| {
                    track
                        .items
                        .get(index + 2)
                        .is_some_and(|item| transition.target_item_id == item.id)
                })
                .map(|transition| transition.duration);
            Some((previous, next))
        }
        crate::project::TrackRef::Caption(_) => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update_transition_drag(
    drag: &mut TransitionDrag,
    project: &Project,
    view: TimelineViewState,
    x: f64,
    snap_repository: &SnapRepo,
) {
    let Some(context) = SequenceTimeline::for_item(project, &drag.key) else {
        return;
    };
    let times = context.timeline_item_times(project, &drag.key);
    let Some((start, end)) = times else {
        return;
    };
    let start_x =
        timeline_x() + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x = timeline_x() + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    drag.remove = x <= start_x || x >= end_x;
    if drag.remove {
        return;
    }
    let target = Time::from_seconds_f64(x_to_time(x, view.scroll_seconds, view.seconds_per_pixel));
    let target = snap_repository.snap(target).unwrap_or(target);
    let (intro, outro) = context
        .transition_durations(project, &drag.key)
        .unwrap_or_default();
    let Some((item_start, item_end)) = project.item(&drag.key).map(|item| item.times()) else {
        return;
    };
    let Some(target) = project
        .timeline_time_to_sequence(&drag.key.track(), target)
        .map(|target| target.snapped(project.frame_step()))
    else {
        return;
    };
    let item_duration = item_end.saturating_sub(item_start);
    let other = match drag.side {
        TransitionSide::Intro => outro,
        TransitionSide::Outro => intro,
    }
    .unwrap_or(Time::ZERO);
    let available = item_duration.saturating_sub(other);
    let duration = match drag.side {
        TransitionSide::Intro => target.saturating_sub(item_start),
        TransitionSide::Outro => item_end.saturating_sub(target),
    };
    drag.target_duration = duration.clamp(Time::ZERO, available);
    drag.target_timeline_duration = context
        .timeline_transition_duration(project, &drag.key, drag.side, drag.target_duration)
        .unwrap_or(Time::ZERO);
}
