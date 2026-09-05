use super::*;
use crate::items::DraggedGroup;

use crate::drop_area;

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

pub(super) fn update(runtime: &mut Scene, project: &Project, position: glam::DVec2) -> bool {
    let update = DragUpdate {
        project,
        view: runtime.view,
        position,
    };
    update_nested(runtime, &update)
        || update_root(runtime, &update)
        || update_resize(runtime, &update)
}

pub(super) fn finish(
    runtime: &mut Scene,
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
    finish_nested(runtime, &finish)
        || finish_root(runtime, &finish)
        || finish_resize(runtime, &finish)
}

fn update_nested(runtime: &mut Scene, update: &DragUpdate<'_>) -> bool {
    let Some(drag) = runtime.folded_drag.as_mut() else {
        return false;
    };
    crate::dragging::update_nested(
        drag,
        update.project,
        crate::dragging::DragRequest {
            view: update.view,
            position: update.position,
            collision_mode: runtime.drag_collision_mode,
            snap_repository: &runtime.snap_repository,
        },
    );
    true
}

fn finish_nested(runtime: &mut Scene, finish: &DragFinish<'_>) -> bool {
    let Some(mut drag) = runtime.folded_drag.take() else {
        return false;
    };
    let resized = !matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move);
    if !finish.moved {
        return true;
    }
    let kind = crate::folded_sequence::track_kind(&drag.key);
    let mut project = finish.project.borrow_mut();
    let waveform_signature = items::audio_waveform_cache_signature(&project);
    let selected = crate::dragging::finish_nested(
        &mut drag,
        &mut project,
        crate::dragging::DragRequest {
            view: finish.view,
            position: finish.position,
            collision_mode: runtime.drag_collision_mode,
            snap_repository: &runtime.snap_repository,
        },
    );
    let Some(outcome) = selected else {
        return true;
    };
    let audio_waveforms = waveform_signature != items::audio_waveform_cache_signature(&project);
    let duration = project.duration();
    project.normalize_clip_transitions();
    crate::project::commit_edit(&project, outcome.commit_name);
    let selected = outcome.selected;
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
    refresh(
        finish.player_state,
        kind,
        duration,
        resized || audio_waveforms,
    );
    true
}

fn update_root(runtime: &mut Scene, update: &DragUpdate<'_>) -> bool {
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
    drop_area::update_root_preview(
        drop_area::DragDropContext {
            project: update.project,
            view: update.view,
            position: update.position,
            collision_mode: runtime.drag_collision_mode,
        },
        group,
    );
    true
}

fn finish_root(runtime: &mut Scene, finish: &DragFinish<'_>) -> bool {
    let Some(mut group) = runtime.dragged_group.take() else {
        return false;
    };
    if !finish.moved {
        if !runtime.modifiers.ctrl && !runtime.modifiers.shift {
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
    if let Some(item_drop) = drop_area::root_item_drop(
        drop_area::DragDropContext {
            project: &project,
            view: finish.view,
            position: finish.position,
            collision_mode: runtime.drag_collision_mode,
        },
        &group,
    ) {
        let kind = group.grabbed.kind;
        drop(project);
        return apply_item_drop(finish, item_drop, kind);
    }
    if !group.valid_drop {
        return true;
    }
    let focused_identity = item_identity(&project, group.grabbed);
    let waveform_signature = items::audio_waveform_cache_signature(&project);
    let Some(selection) = move_dragged_group(&mut project, &group) else {
        return true;
    };
    let focused_item = focused_identity.and_then(|item| item_key_for_identity(&project, item));
    let duration = project.duration();
    let audio_waveforms = waveform_signature != items::audio_waveform_cache_signature(&project);
    let change = group_change(&group, duration, audio_waveforms);
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

fn apply_item_drop(
    finish: &DragFinish<'_>,
    item_drop: drop_area::ItemDrop,
    kind: TrackKind,
) -> bool {
    let mut project = finish.project.borrow_mut();
    let waveform_signature = items::audio_waveform_cache_signature(&project);
    let Some(moved) = item_drop.apply(&mut project) else {
        return true;
    };
    let duration = project.duration();
    let audio_waveforms = waveform_signature != items::audio_waveform_cache_signature(&project);
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
    refresh(finish.player_state, kind, duration, audio_waveforms);
    true
}

fn update_resize(runtime: &mut Scene, update: &DragUpdate<'_>) -> bool {
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

fn finish_resize(runtime: &mut Scene, finish: &DragFinish<'_>) -> bool {
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
        mark_change(&mut change, key.kind, true);
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

fn refresh(
    player_state: &SharedPlayerState,
    kind: TrackKind,
    duration: Time,
    audio_waveforms: bool,
) {
    let mut change = ProjectChange {
        duration: Some(duration),
        inspector: true,
        ..ProjectChange::default()
    };
    mark_change(&mut change, kind, audio_waveforms);
    player_state::refresh_project(player_state, change);
}

fn group_change(group: &DraggedGroup, duration: Time, audio_waveforms: bool) -> ProjectChange {
    let mut change = ProjectChange {
        duration: Some(duration),
        ..ProjectChange::default()
    };
    for item in &group.items {
        mark_change(&mut change, item.key.kind, audio_waveforms);
    }
    change
}

fn mark_change(change: &mut ProjectChange, kind: TrackKind, audio_waveforms: bool) {
    match kind {
        TrackKind::Caption => change.captions = true,
        TrackKind::Video => change.video = true,
        TrackKind::Audio => {
            change.audio = true;
            change.audio_waveforms |= audio_waveforms;
        }
    }
}
