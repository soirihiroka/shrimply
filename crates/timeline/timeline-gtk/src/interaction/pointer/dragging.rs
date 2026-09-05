use super::*;
use crate::items::{DragCollisionMode, DraggedGroup};
use crate::project::ItemAddress;
use crate::timeline_operation::SequenceTimeline;

mod drop_area;

struct DragUpdate<'a> {
    project: &'a Project,
    view: TimelineViewState,
    position: glam::DVec2,
}

struct DragFinish<'a> {
    project: &'a Rc<RefCell<Project>>,
    player_state: &'a SharedPlayerState,
    selection_state: &'a SharedSelectionState,
    view: TimelineViewState,
    position: glam::DVec2,
    moved: bool,
}

trait Draggable {
    fn update(&self, runtime: &mut TimelineRuntime, update: &DragUpdate<'_>) -> bool;
    fn finish(&self, runtime: &mut TimelineRuntime, finish: &DragFinish<'_>) -> bool;
}

struct NestedItem;
struct RootItems;
struct RootResize;

pub(super) fn update(
    runtime: &mut TimelineRuntime,
    project: &Project,
    position: glam::DVec2,
) -> bool {
    let update = DragUpdate {
        project,
        view: runtime.view,
        position,
    };
    let handlers: [&dyn Draggable; 3] = [&NestedItem, &RootItems, &RootResize];
    handlers
        .into_iter()
        .any(|handler| handler.update(runtime, &update))
}

pub(super) fn finish(
    runtime: &mut TimelineRuntime,
    project: &Rc<RefCell<Project>>,
    player_state: &SharedPlayerState,
    selection_state: &SharedSelectionState,
    position: glam::DVec2,
) -> bool {
    let finish = DragFinish {
        project,
        player_state,
        selection_state,
        view: runtime.view,
        position,
        moved: runtime.view.drag_moved,
    };
    let handlers: [&dyn Draggable; 3] = [&NestedItem, &RootItems, &RootResize];
    handlers
        .into_iter()
        .any(|handler| handler.finish(runtime, &finish))
}

impl Draggable for NestedItem {
    fn update(&self, runtime: &mut TimelineRuntime, update: &DragUpdate<'_>) -> bool {
        let Some(drag) = runtime.folded_drag.as_mut() else {
            return false;
        };
        let previous = (
            drag.target_start,
            drag.target_end,
            drag.target_track.clone(),
            drag.items.clone(),
        );
        let context = SequenceTimeline::for_item(update.project, &drag.key)
            .expect("dragged item must have a valid operation scope");
        drag.cross_scope_preview_row = None;
        if matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move)
            && let Some(drop) = drop_area::ItemDrop::at(
                drop_area::DragDropContext {
                    project: update.project,
                    view: update.view,
                    position: update.position,
                    collision_mode: runtime.drag_collision_mode,
                },
                &context,
                &drag.key,
                (drag.target_start, drag.target_end),
            )
        {
            if let Some(target) = drop.target_track().cloned() {
                let _ = drag.set_target_track(&context, update.project, target);
            } else if drop.is_nested() {
                let content_y = update.position.y + update.view.scroll_y - RULER_HEIGHT;
                drag.cross_scope_preview_row =
                    (content_y >= 0.0).then(|| (content_y / TRACK_HEIGHT).floor() as usize);
            }
        }
        let target = Time::from_seconds_f64(x_to_time(
            update.position.x,
            update.view.scroll_seconds,
            update.view.seconds_per_pixel,
        ));
        let target = if drag.kind == crate::folded_sequence::FoldedDragKind::Move {
            target.signed_sub(drag.pointer_offset)
        } else {
            target
        };
        let target = runtime.snap_repository.snap(target).unwrap_or(target);
        crate::folded_sequence::update_drag(
            drag,
            target,
            crate::geometry::frame_step(update.project),
        );
        if matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move) {
            let drop = drop_area::ItemDrop::at(
                drop_area::DragDropContext {
                    project: update.project,
                    view: update.view,
                    position: update.position,
                    collision_mode: runtime.drag_collision_mode,
                },
                &context,
                &drag.key,
                (drag.target_start, drag.target_end),
            );
            drag.valid_drop = drop.as_ref().is_some_and(|drop| {
                if drop.target_track().is_none() {
                    drag.items.len() == 1 && drop.can_apply(update.project)
                } else {
                    let mut candidate = update.project.clone();
                    crate::folded_sequence::apply_move_drag(
                        &mut candidate,
                        drag,
                        runtime.drag_collision_mode,
                    )
                    .is_some()
                }
            });
            drag.preview_status = if !drag.valid_drop {
                crate::items::DragPreviewStatus::Blocked
            } else {
                match runtime.drag_collision_mode {
                    DragCollisionMode::Overwrite => crate::items::DragPreviewStatus::Overwrite,
                    DragCollisionMode::Block => crate::items::DragPreviewStatus::Clear,
                    DragCollisionMode::NewTrack => crate::items::DragPreviewStatus::NewTrack,
                }
            };
        }
        if runtime.drag_collision_mode == DragCollisionMode::Block
            && !crate::folded_sequence::can_apply_drag(update.project, drag)
        {
            (
                drag.target_start,
                drag.target_end,
                drag.target_track,
                drag.items,
            ) = previous;
        }
        true
    }

    fn finish(&self, runtime: &mut TimelineRuntime, finish: &DragFinish<'_>) -> bool {
        let Some(mut drag) = runtime.folded_drag.take() else {
            return false;
        };
        if !finish.moved {
            return true;
        }
        let context = {
            let project = finish.project.borrow();
            SequenceTimeline::for_item(&project, &drag.key)
                .expect("dragged item must have a valid operation scope")
        };
        let target = {
            let target = Time::from_seconds_f64(x_to_time(
                finish.position.x,
                finish.view.scroll_seconds,
                finish.view.seconds_per_pixel,
            ));
            let target = if drag.kind == crate::folded_sequence::FoldedDragKind::Move {
                target.signed_sub(drag.pointer_offset)
            } else {
                target
            };
            runtime.snap_repository.snap(target).unwrap_or(target)
        };
        crate::folded_sequence::update_drag(
            &mut drag,
            target,
            crate::geometry::frame_step(&finish.project.borrow()),
        );
        if matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move) {
            let item_drop = {
                let project = finish.project.borrow();
                drop_area::ItemDrop::at(
                    drop_area::DragDropContext {
                        project: &project,
                        view: finish.view,
                        position: finish.position,
                        collision_mode: runtime.drag_collision_mode,
                    },
                    &context,
                    &drag.key,
                    (drag.target_start, drag.target_end),
                )
            };
            let Some(item_drop) = item_drop else {
                return true;
            };
            if item_drop.target_track().is_none() {
                if drag.items.len() != 1 {
                    return true;
                }
                let kind = crate::folded_sequence::track_kind(&drag.key);
                return apply_item_drop(finish, item_drop, kind);
            }
            let target_track = item_drop
                .target_track()
                .expect("existing-track drop must expose its target")
                .clone();
            let project = finish.project.borrow();
            if !drag.set_target_track(&context, &project, target_track) {
                return true;
            }
        }
        let kind = crate::folded_sequence::track_kind(&drag.key);
        let mut project = finish.project.borrow_mut();
        let selected = if drag.kind == crate::folded_sequence::FoldedDragKind::Move {
            crate::folded_sequence::apply_move_drag(
                &mut project,
                &drag,
                runtime.drag_collision_mode,
            )
        } else {
            crate::folded_sequence::apply_resize_drag(
                &mut project,
                &drag,
                match runtime.drag_collision_mode {
                    DragCollisionMode::NewTrack => DragCollisionMode::Block,
                    mode => mode,
                },
            )
        };
        let Some(selected) = selected else {
            return true;
        };
        let duration = project.duration();
        project.normalize_clip_transitions();
        crate::project::commit_edit(
            &project,
            if drag.kind == crate::folded_sequence::FoldedDragKind::Move {
                "move-timeline-items"
            } else {
                "resize-timeline-items"
            },
        );
        drop(project);
        let project = finish.project.borrow();
        let focused = selected
            .iter()
            .find(|address| address.item_id() == drag.key.item_id())
            .cloned();
        selection_state::set_selected_item_addresses(
            finish.selection_state,
            &project,
            selected,
            focused,
        );
        drop(project);
        refresh(finish.player_state, kind, duration);
        true
    }
}

impl Draggable for RootItems {
    fn update(&self, runtime: &mut TimelineRuntime, update: &DragUpdate<'_>) -> bool {
        let Some(group) = runtime.dragged_group.as_mut() else {
            return false;
        };
        update_dragged_group(
            group,
            update.project,
            update.view,
            update.position.x,
            update.position.y,
            &runtime.snap_repository,
        );
        if let Some(source) = single_root_address(update.project, group)
            && let Some(item) = group.items.first()
            && let Some(times) = crate::target_item_times(group, item)
            && let Some(drop) = drop_area::ItemDrop::at(
                drop_area::DragDropContext {
                    project: update.project,
                    view: update.view,
                    position: update.position,
                    collision_mode: runtime.drag_collision_mode,
                },
                &SequenceTimeline::root(),
                &source,
                times,
            )
            && drop.is_nested()
        {
            group.valid_drop = drop.can_apply(update.project);
            group.preview_status = if !group.valid_drop {
                crate::items::DragPreviewStatus::Blocked
            } else {
                match runtime.drag_collision_mode {
                    DragCollisionMode::Overwrite => crate::items::DragPreviewStatus::Overwrite,
                    DragCollisionMode::Block => crate::items::DragPreviewStatus::Clear,
                    DragCollisionMode::NewTrack => crate::items::DragPreviewStatus::NewTrack,
                }
            };
            let content_y = update.position.y + update.view.scroll_y - RULER_HEIGHT;
            group.cross_scope_preview_row =
                (content_y >= 0.0).then(|| (content_y / TRACK_HEIGHT).floor() as usize);
            group.new_tracks.clear();
            group.blocked_indicators.clear();
            group.overwrite_indicators.clear();
        }
        true
    }

    fn finish(&self, runtime: &mut TimelineRuntime, finish: &DragFinish<'_>) -> bool {
        let Some(mut group) = runtime.dragged_group.take() else {
            return false;
        };
        if !finish.moved {
            if !runtime.modifiers.ctrl && selected_timeline_items(finish.selection_state).len() > 1
            {
                let project = finish.project.borrow();
                set_selection(
                    &project,
                    finish.selection_state,
                    vec![group.grabbed],
                    Some(group.grabbed),
                    true,
                );
            }
            return true;
        }
        {
            let project = finish.project.borrow();
            update_dragged_group(
                &mut group,
                &project,
                finish.view,
                finish.position.x,
                finish.position.y,
                &runtime.snap_repository,
            );
        }
        let mut project = finish.project.borrow_mut();
        if let Some(source) = single_root_address(&project, &group)
            && let Some(item) = group.items.first()
            && let Some((start, end)) = crate::target_item_times(&group, item)
            && let Some(item_drop) = drop_area::ItemDrop::at(
                drop_area::DragDropContext {
                    project: &project,
                    view: finish.view,
                    position: finish.position,
                    collision_mode: runtime.drag_collision_mode,
                },
                &SequenceTimeline::root(),
                &source,
                (start, end),
            )
        {
            let kind = group.grabbed.kind;
            drop(project);
            return apply_item_drop(finish, item_drop, kind);
        }
        if !group.valid_drop {
            return true;
        }
        let focused_identity = item_identity(&project, group.grabbed);
        let Some(selection) = move_dragged_group(&mut project, &group) else {
            return true;
        };
        let focused_item = focused_identity.and_then(|item| item_key_for_identity(&project, item));
        let duration = project.duration();
        let change = group_change(&group, duration);
        project.normalize_clip_transitions();
        crate::project::commit_edit(&project, "move-timeline-items");
        drop(project);
        let project = finish.project.borrow();
        set_selection(
            &project,
            finish.selection_state,
            selection,
            focused_item,
            false,
        );
        player_state::refresh_project(finish.player_state, change);
        true
    }
}

fn apply_item_drop(
    finish: &DragFinish<'_>,
    item_drop: drop_area::ItemDrop,
    kind: TrackKind,
) -> bool {
    let mut project = finish.project.borrow_mut();
    let Some(moved) = item_drop.apply(&mut project) else {
        return true;
    };
    let duration = project.duration();
    project.normalize_clip_transitions();
    crate::project::commit_edit(&project, "move-timeline-item-between-tracks");
    drop(project);
    let project = finish.project.borrow();
    selection_state::set_selected_item_addresses(
        finish.selection_state,
        &project,
        vec![moved.clone()],
        Some(moved),
    );
    drop(project);
    refresh(finish.player_state, kind, duration);
    true
}

impl Draggable for RootResize {
    fn update(&self, runtime: &mut TimelineRuntime, update: &DragUpdate<'_>) -> bool {
        let Some(resize) = runtime.resize_drag.as_mut() else {
            return false;
        };
        update_resize_drag(
            resize,
            update.project,
            update.view,
            update.position.x,
            &runtime.snap_repository,
        );
        true
    }

    fn finish(&self, runtime: &mut TimelineRuntime, finish: &DragFinish<'_>) -> bool {
        let Some(mut resize) = runtime.resize_drag.take() else {
            return false;
        };
        if !finish.moved {
            return true;
        }
        let project = finish.project.borrow();
        update_resize_drag(
            &mut resize,
            &project,
            finish.view,
            finish.position.x,
            &runtime.snap_repository,
        );
        drop(project);
        if !resize.valid {
            return true;
        }
        let mut project = finish.project.borrow_mut();
        let focused_identity = item_identity(&project, resize.key);
        let Some(selection) = apply_resize_drag(&mut project, resize) else {
            return true;
        };
        let focused_item = focused_identity.and_then(|item| item_key_for_identity(&project, item));
        let duration = project.duration();
        let mut change = ProjectChange {
            duration: Some(duration),
            ..ProjectChange::default()
        };
        for key in &selection {
            mark_change(&mut change, key.kind);
        }
        project.normalize_clip_transitions();
        crate::project::commit_edit(&project, "resize-timeline-item");
        drop(project);
        let project = finish.project.borrow();
        set_selection(
            &project,
            finish.selection_state,
            selection,
            focused_item,
            true,
        );
        player_state::refresh_project(finish.player_state, change);
        true
    }
}

fn single_root_address(project: &Project, group: &DraggedGroup) -> Option<ItemAddress> {
    (group.items.len() == 1).then(|| selection_state::item_address(project, group.items[0].key))?
}

fn refresh(player_state: &SharedPlayerState, kind: TrackKind, duration: Time) {
    let mut change = ProjectChange {
        duration: Some(duration),
        inspector: true,
        ..ProjectChange::default()
    };
    mark_change(&mut change, kind);
    player_state::refresh_project(player_state, change);
}

fn group_change(group: &DraggedGroup, duration: Time) -> ProjectChange {
    let mut change = ProjectChange {
        duration: Some(duration),
        ..ProjectChange::default()
    };
    for item in &group.items {
        mark_change(&mut change, item.key.kind);
    }
    change
}

fn mark_change(change: &mut ProjectChange, kind: TrackKind) {
    match kind {
        TrackKind::Caption => change.captions = true,
        TrackKind::Video => change.video = true,
        TrackKind::Audio => {
            change.audio = true;
            change.audio_waveforms = true;
        }
    }
}
