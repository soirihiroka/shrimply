use crate::{
    geometry::timeline_x,
    items::{ItemKey, TrackKind, hit_item_at},
    project::{CaptionItem, Project, Time},
    view::TimelineViewState,
};
use shrimply_timeline_snap::SnapRepo;

/// Create the GTK double-click caption on a candidate project. The host commits.
pub fn insert(
    project: &mut Project,
    point: glam::DVec2,
    view: TimelineViewState,
    snap_repository: &SnapRepo,
    default_duration: Time,
) -> Option<ItemKey> {
    let (x, y) = (point.x, point.y);
    if x < timeline_x() {
        return None;
    }
    let project_state = &*project;
    let time = crate::math::time_at_x(view, x);
    let snapped_time = snap_repository.snap(time).unwrap_or(time);
    let track_info = crate::items::track_at_y(project_state, y + view.scroll_y);
    let has_hit = hit_item_at(project_state, view, x, y).is_some();
    if has_hit {
        return None;
    }
    let (kind, track_index, _) = track_info?;
    if !matches!(kind, TrackKind::Caption) {
        return None;
    }
    let mut end = snapped_time
        .saturating_add(default_duration)
        .snapped(project_state.frame_step());
    let track = project_state.caption_tracks.get(track_index)?;
    for item in &track.items {
        if item.start <= snapped_time && snapped_time < item.end {
            return None;
        }
        if item.start > snapped_time {
            end = end.min(item.start);
            break;
        }
    }
    if end <= snapped_time {
        return None;
    }
    let track = project.caption_tracks.get_mut(track_index)?;
    let item_index = crate::items::insert_sorted(
        &mut track.items,
        CaptionItem::new(snapped_time, end, String::new()),
    );
    let item_key = ItemKey {
        kind: TrackKind::Caption,
        track_index,
        item_index,
    };
    Some(item_key)
}
