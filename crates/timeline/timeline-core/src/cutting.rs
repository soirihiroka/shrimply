use crate::items::{cut_time_for_address, hit_item_at};
use crate::selection_state;
use crate::{
    TimelineCut,
    project::{Project, Time},
    timeline_operation::SequenceTimeline,
    view::TimelineViewState,
};
use shrimply_timeline_snap::SnapRepo;

pub fn timeline_cut(
    project: &Project,
    selected_items: &[crate::project::ItemAddress],
    key: crate::project::ItemAddress,
    time: Time,
) -> TimelineCut {
    let seed = if selected_items.contains(&key) {
        selected_items
    } else {
        std::slice::from_ref(&key)
    };
    let context = SequenceTimeline::for_item(project, &key)
        .expect("cut item must have a valid operation scope");
    let mut keys = crate::items::expand_grouped_item_addresses(&context, project, seed);
    if !keys.contains(&key) {
        keys.push(key.clone());
    }
    keys.sort_by_key(|address| (address.track_id(), address.item_id()));
    keys.dedup();
    TimelineCut { key, time, keys }
}

pub struct PreviewRequest<'a> {
    pub view: TimelineViewState,
    pub position: glam::DVec2,
    pub active: bool,
    pub snaps: &'a SnapRepo,
}

pub fn preview(
    project: &Project,
    selected: &[crate::project::ItemAddress],
    previous: Option<&TimelineCut>,
    request: PreviewRequest<'_>,
) -> Option<TimelineCut> {
    let (x, y) = (request.position.x, request.position.y);
    let hit = crate::folded_sequence::hit_projected_item(project, request.view, x, y)
        .map(|hit| hit.key)
        .or_else(|| {
            hit_item_at(project, request.view, x, y)
                .and_then(|key| selection_state::item_address(project, key))
        })?;
    if request.active && previous.is_some_and(|preview| !preview.keys.contains(&hit)) {
        return None;
    }
    let time = cut_time_for_address(project, request.view, &hit, x, request.snaps)?;
    Some(timeline_cut(project, selected, hit, time))
}
