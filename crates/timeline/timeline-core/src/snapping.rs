use hashbrown::HashSet;

use crate::folded_sequence::FoldedDragKind;
use crate::items::{
    ItemEdge, item_natural_end_edges_at_address, item_natural_resize_candidates_at_address,
    item_natural_snap_targets_at_address,
};
use crate::project::{ItemAddress, Project, SequenceScopeId, Time};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};
pub use shrimply_timeline_snap::SnapRepo;
use shrimply_timeline_snap::{SnapSources, TLSnappable};

enum TimelineSnappable<'a> {
    Globals {
        duration: Time,
        beats: Vec<Time>,
        playhead: Time,
    },
    Item {
        project: &'a Project,
        address: ItemAddress,
    },
}

impl TLSnappable for TimelineSnappable<'_> {
    fn snap_times(self) -> Vec<Time> {
        match self {
            Self::Globals {
                duration,
                mut beats,
                playhead,
            } => {
                beats.extend([Time::ZERO, duration, playhead]);
                beats
            }
            Self::Item { project, address } => {
                let mut targets = project
                    .timeline_item_times(&address)
                    .map(|(start, end)| vec![start, end])
                    .unwrap_or_default();
                targets.extend(item_natural_snap_targets_at_address(project, &address));
                targets
            }
        }
    }
}

pub struct Request<'a> {
    pub folded_drag: Option<&'a crate::folded_sequence::FoldedDrag>,
    pub dragged_group: Option<&'a crate::items::DraggedGroup>,
    pub resize_drag: Option<&'a crate::items::ResizeDrag>,
    pub beats: Vec<Time>,
    pub playhead: Time,
    pub distance: Option<Time>,
}

pub fn repository(project: &Project, request: Request<'_>) -> SnapRepo {
    let mut ignored = HashSet::new();
    let mut offsets = Vec::new();
    let mut candidates = Vec::new();
    let scope = if let Some(drag) = request.folded_drag {
        let origin = match drag.kind {
            FoldedDragKind::Move | FoldedDragKind::ResizeStart => drag.start,
            FoldedDragKind::ResizeEnd => drag.end,
        };
        for item in &drag.items {
            ignored.insert(item.key.item_id());
            let edge = match drag.kind {
                FoldedDragKind::Move | FoldedDragKind::ResizeStart => item.start,
                FoldedDragKind::ResizeEnd => item.end,
            };
            let offset = edge.signed_sub(origin);
            offsets.push(offset);
            if drag.kind == FoldedDragKind::Move {
                offsets.push(item.end.signed_sub(origin));
                offsets.extend(
                    item_natural_end_edges_at_address(project, &item.key)
                        .into_iter()
                        .map(|target| target.signed_sub(origin)),
                );
            } else {
                candidates.extend(
                    item_natural_resize_candidates_at_address(project, &item.key)
                        .into_iter()
                        .map(|target| target.signed_sub(offset)),
                );
            }
        }
        project.item_scope(&drag.key).unwrap_or_default()
    } else if let Some(group) = request.dragged_group {
        for item in &group.items {
            let Some(address) = crate::selection_state::item_address(project, item.key) else {
                continue;
            };
            ignored.insert(address.item_id());
            offsets.extend([
                item.start.signed_sub(group.grabbed_start),
                item.end.signed_sub(group.grabbed_start),
            ]);
            offsets.extend(
                item_natural_end_edges_at_address(project, &address)
                    .into_iter()
                    .map(|target| target.signed_sub(group.grabbed_start)),
            );
        }
        SequenceScopeId::root()
    } else if let Some(drag) = request.resize_drag {
        let origin = match drag.edge {
            ItemEdge::Start => drag.start,
            ItemEdge::End => drag.end,
        };
        for item in &drag.items {
            let Some(address) = crate::selection_state::item_address(project, item.key) else {
                continue;
            };
            ignored.insert(address.item_id());
            let offset = match drag.edge {
                ItemEdge::Start => item.start,
                ItemEdge::End => item.end,
            }
            .signed_sub(origin);
            offsets.push(offset);
            candidates.extend(
                item_natural_resize_candidates_at_address(project, &address)
                    .into_iter()
                    .map(|target| target.signed_sub(offset)),
            );
        }
        SequenceScopeId::root()
    } else {
        offsets.push(Time::ZERO);
        SequenceScopeId::root()
    };

    let mut snappables = vec![TimelineSnappable::Globals {
        duration: project.duration(),
        beats: request.beats,
        playhead: request.playhead,
    }];
    for address in SequenceTimeline::new(scope).items(project) {
        if !ignored.contains(&address.item_id()) {
            snappables.push(TimelineSnappable::Item { project, address });
        }
    }
    SnapRepo::new(SnapSources {
        snappables,
        candidates,
        offsets,
        frame_step: project.frame_step(),
        distance: request.distance,
    })
}
