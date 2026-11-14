use hashbrown::HashSet;

use crate::folded_sequence::FoldedDragKind;
use crate::items::{
    ItemEdge, item_natural_end_edges_at_address, item_natural_resize_candidates_at_address,
    item_natural_snap_targets_for_item,
};
use crate::project::{ItemRef, Project, SequenceScopeId, Time, TrackAddress};
use crate::timeline_operation::{SequenceTimeline, TimelineOperationContext};
pub use shrimply_timeline_snap::SnapRepo;
use shrimply_timeline_snap::{SnapQuery, SnapSources, TLSnappable};

enum TimelineSnappable<'a> {
    Globals {
        duration: Time,
        beats: Vec<Time>,
    },
    Item {
        project: &'a Project,
        track: TrackAddress,
        item: ItemRef<'a>,
    },
}

impl TLSnappable for TimelineSnappable<'_> {
    fn snap_times(self) -> Vec<Time> {
        match self {
            Self::Globals {
                duration,
                mut beats,
            } => {
                beats.extend([Time::ZERO, duration]);
                beats
            }
            Self::Item {
                project,
                track,
                item,
            } => {
                let (start, end) = item.times();
                let mut targets = [start, end]
                    .into_iter()
                    .filter_map(|time| project.sequence_time_to_timeline(&track, time))
                    .collect::<Vec<_>>();
                targets.extend(item_natural_snap_targets_for_item(project, &track, &item));
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

#[derive(Default)]
pub struct Cache {
    revision: Option<u64>,
    scope: Option<SequenceScopeId>,
    ignored: HashSet<uuid::Uuid>,
    beats: Vec<Time>,
}

pub fn update(
    project: &Project,
    revision: u64,
    cache: &mut Cache,
    repository: &mut SnapRepo,
    request: Request<'_>,
) {
    if request.distance.is_none() {
        repository.update_query(SnapQuery {
            candidates: Vec::new(),
            offsets: Vec::new(),
            distance: None,
            dynamic_time: None,
        });
        return;
    }

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

    if cache.revision != Some(revision)
        || cache.scope.as_ref() != Some(&scope)
        || cache.ignored != ignored
        || cache.beats != request.beats
    {
        cache.revision = Some(revision);
        cache.scope = Some(scope.clone());
        cache.ignored = ignored;
        cache.beats = request.beats;
        let mut snappables = vec![TimelineSnappable::Globals {
            duration: project.duration(),
            beats: cache.beats.clone(),
        }];
        for track in SequenceTimeline::new(scope).tracks(project) {
            if let Some(items) = project.track(&track) {
                for item in items.items() {
                    if !cache.ignored.contains(&item.id()) {
                        snappables.push(TimelineSnappable::Item {
                            project,
                            track: track.clone(),
                            item,
                        });
                    }
                }
            }
        }
        *repository = SnapRepo::new(SnapSources {
            snappables,
            candidates: Vec::new(),
            offsets: Vec::new(),
            frame_step: project.frame_step(),
            distance: request.distance,
        });
    }
    repository.update_query(SnapQuery {
        candidates,
        offsets,
        distance: request.distance,
        dynamic_time: Some(request.playhead),
    });
}
