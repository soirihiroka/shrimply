use crate::drop_area;
use crate::folded_sequence::{self, FoldedDrag, FoldedDragKind};
use crate::items::DragCollisionMode;
use crate::metrics::{RULER_HEIGHT, TRACK_HEIGHT};
use crate::project::{ItemAddress, Project};
use crate::timeline_operation::SequenceTimeline;
use crate::view::TimelineViewState;
use shrimply_timeline_snap::SnapRepo;

#[derive(Clone, Copy)]
pub struct DragRequest<'a> {
    pub view: TimelineViewState,
    pub position: glam::DVec2,
    pub collision_mode: DragCollisionMode,
    pub snap_repository: &'a SnapRepo,
}

pub fn update_nested(drag: &mut FoldedDrag, project: &Project, request: DragRequest<'_>) {
    let previous = (
        drag.target_start,
        drag.target_end,
        drag.target_track.clone(),
        drag.items.clone(),
    );
    let context = SequenceTimeline::for_item(project, &drag.key)
        .expect("dragged item must have a valid operation scope");
    drag.cross_scope_preview_row = None;
    if matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move)
        && let Some(drop) = drop_area::ItemDrop::at(
            drop_area::DragDropContext {
                project,
                view: request.view,
                position: request.position,
                collision_mode: request.collision_mode,
            },
            &context,
            &drag.key,
            (drag.target_start, drag.target_end),
        )
    {
        if let Some(target) = drop.target_track().cloned() {
            let _ = drag.set_target_track(&context, project, target);
        } else if drop.is_nested() {
            let content_y = request.position.y + request.view.scroll_y - RULER_HEIGHT;
            drag.cross_scope_preview_row =
                (content_y >= 0.0).then(|| (content_y / TRACK_HEIGHT).floor() as usize);
        }
    }
    let target = crate::math::time_at_x(request.view, request.position.x);
    let target = if drag.kind == crate::folded_sequence::FoldedDragKind::Move {
        target.signed_sub(drag.pointer_offset)
    } else {
        target
    };
    let target = request.snap_repository.snap(target).unwrap_or(target);
    crate::folded_sequence::update_drag(drag, target, crate::geometry::frame_step(project));
    if matches!(drag.kind, crate::folded_sequence::FoldedDragKind::Move) {
        let drop = drop_area::ItemDrop::at(
            drop_area::DragDropContext {
                project,
                view: request.view,
                position: request.position,
                collision_mode: request.collision_mode,
            },
            &context,
            &drag.key,
            (drag.target_start, drag.target_end),
        );
        drag.valid_drop = drop.as_ref().is_some_and(|drop| {
            if drop.target_track().is_none() {
                drag.items.len() == 1 && drop.can_apply(project)
            } else {
                let mut candidate = project.clone();
                crate::folded_sequence::apply_move_drag(
                    &mut candidate,
                    drag,
                    request.collision_mode,
                )
                .is_some()
            }
        });
        drag.preview_status = if !drag.valid_drop {
            crate::items::DragPreviewStatus::Blocked
        } else {
            match request.collision_mode {
                DragCollisionMode::Overwrite => crate::items::DragPreviewStatus::Overwrite,
                DragCollisionMode::Block => crate::items::DragPreviewStatus::Clear,
                DragCollisionMode::NewTrack => crate::items::DragPreviewStatus::NewTrack,
            }
        };
    }
    if request.collision_mode == DragCollisionMode::Block
        && !crate::folded_sequence::can_apply_drag(project, drag)
    {
        (
            drag.target_start,
            drag.target_end,
            drag.target_track,
            drag.items,
        ) = previous;
    }
}

pub struct NestedDrop {
    pub selected: Vec<ItemAddress>,
    pub commit_name: &'static str,
}

pub fn finish_nested(
    drag: &mut FoldedDrag,
    project: &mut Project,
    request: DragRequest<'_>,
) -> Option<NestedDrop> {
    let context = SequenceTimeline::for_item(project, &drag.key)?;
    let pointer = crate::math::time_at_x(request.view, request.position.x);
    let target = if drag.kind == FoldedDragKind::Move {
        pointer.signed_sub(drag.pointer_offset)
    } else {
        pointer
    };
    let target = request.snap_repository.snap(target).unwrap_or(target);
    folded_sequence::update_drag(drag, target, project.frame_step());
    if drag.kind == FoldedDragKind::Move {
        let item_drop = drop_area::ItemDrop::at(
            drop_area::DragDropContext {
                project,
                view: request.view,
                position: request.position,
                collision_mode: request.collision_mode,
            },
            &context,
            &drag.key,
            (drag.target_start, drag.target_end),
        )?;
        let Some(target) = item_drop.target_track().cloned() else {
            if drag.items.len() != 1 {
                return None;
            }
            return Some(NestedDrop {
                selected: vec![item_drop.apply(project)?],
                commit_name: "move-timeline-item-between-tracks",
            });
        };
        if !drag.set_target_track(&context, project, target) {
            return None;
        }
        Some(NestedDrop {
            selected: folded_sequence::apply_move_drag(project, drag, request.collision_mode)?,
            commit_name: "move-timeline-items",
        })
    } else {
        Some(NestedDrop {
            selected: folded_sequence::apply_resize_drag(
                project,
                drag,
                match request.collision_mode {
                    DragCollisionMode::NewTrack => DragCollisionMode::Block,
                    mode => mode,
                },
            )?,
            commit_name: "resize-timeline-items",
        })
    }
}
