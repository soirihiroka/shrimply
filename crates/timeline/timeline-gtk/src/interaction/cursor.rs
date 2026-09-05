use super::*;

pub(crate) fn timeline_cursor(
    project: &Project,
    runtime: &TimelineRuntime,
    x: f64,
    y: f64,
) -> TimelineCursor {
    match runtime.view.drag_mode {
        DragMode::ResizeItem => runtime
            .resize_drag
            .as_ref()
            .map(|resize| resize.edge)
            .or_else(|| {
                runtime
                    .folded_drag
                    .as_ref()
                    .and_then(|drag| match drag.kind {
                        crate::folded_sequence::FoldedDragKind::ResizeStart => {
                            Some(ItemEdge::Start)
                        }
                        crate::folded_sequence::FoldedDragKind::ResizeEnd => Some(ItemEdge::End),
                        crate::folded_sequence::FoldedDragKind::Move => None,
                    })
            })
            .map_or(TimelineCursor::Default, item_resize_cursor),
        DragMode::Transition => {
            if runtime
                .clip_transition_drag
                .as_ref()
                .is_some_and(|drag| drag.center_resize)
            {
                TimelineCursor::ResizeHorizontal
            } else if runtime.transition_drag.is_some() || runtime.clip_transition_drag.is_some() {
                TimelineCursor::Crosshair
            } else {
                TimelineCursor::Default
            }
        }
        DragMode::None => hit_clip_transition_at(project, runtime.view, x, y)
            .and_then(|hit| match hit.action {
                ClipTransitionHitAction::Body => None,
                ClipTransitionHitAction::CenterHandle => Some(TimelineCursor::ResizeHorizontal),
                ClipTransitionHitAction::Create
                | ClipTransitionHitAction::StartHandle
                | ClipTransitionHitAction::EndHandle => Some(TimelineCursor::Crosshair),
            })
            .or_else(|| {
                hit_transition_at(project, runtime.view, x, y)
                    .filter(|hit| !matches!(hit.action, TransitionHitAction::Body))
                    .map(|_| TimelineCursor::Crosshair)
            })
            .or_else(|| {
                crate::folded_sequence::hit_projected_item(project, runtime.view, x, y)
                    .and_then(|hit| {
                        let (item_x, item_width) =
                            crate::item_rect(hit.start, hit.end, timeline_x(), runtime.view);
                        if x <= item_x + ITEM_RESIZE_HANDLE_WIDTH {
                            Some(TimelineCursor::ResizeStart)
                        } else if x >= item_x + item_width - ITEM_RESIZE_HANDLE_WIDTH {
                            Some(TimelineCursor::ResizeEnd)
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        hit_resize_handle_at(project, runtime.view, x, y, ITEM_RESIZE_HANDLE_WIDTH)
                            .map(|(_, edge)| item_resize_cursor(edge))
                    })
            })
            .unwrap_or(TimelineCursor::Default),
        _ => TimelineCursor::Default,
    }
}

fn item_resize_cursor(edge: ItemEdge) -> TimelineCursor {
    match edge {
        ItemEdge::Start => TimelineCursor::ResizeStart,
        ItemEdge::End => TimelineCursor::ResizeEnd,
    }
}
