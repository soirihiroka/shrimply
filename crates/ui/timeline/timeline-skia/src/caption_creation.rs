use crate::{
    geometry::timeline_x,
    items::{ItemKey, TrackKind, hit_item_at},
    project::{CaptionItem, CommentItem, Project, Time},
    view::TimelineViewState,
};
use shrimply_timeline_snap::SnapRepo;

/// Create a text item on double-click. The host commits the candidate project.
pub fn insert(
    project: &mut Project,
    point: glam::DVec2,
    view: TimelineViewState,
    snap_repository: &SnapRepo,
    default_duration: Time,
) -> Option<ItemKey> {
    let (x, y) = (point.x, point.y);
    if x < timeline_x(view) {
        return None;
    }
    let project_state = &*project;
    let time = crate::math::time_at_x(view, x);
    let snapped_time = snap_repository
        .snap(time)
        .unwrap_or(time)
        .snapped(project_state.frame_step());
    let track_info = crate::items::track_at_y(project_state, y + view.scroll_y);
    let has_hit = hit_item_at(project_state, view, x, y).is_some();
    if has_hit {
        return None;
    }
    let (kind, track_index, _) = track_info?;
    if !matches!(kind, TrackKind::Caption | TrackKind::Comment) {
        return None;
    }
    let mut end = snapped_time
        .saturating_add(default_duration)
        .snapped(project_state.frame_step());
    let times: Vec<_> = match kind {
        TrackKind::Comment => project_state
            .comment_tracks
            .get(track_index)?
            .items
            .iter()
            .map(|item| (item.start, item.end))
            .collect(),
        TrackKind::Caption => project_state
            .caption_tracks
            .get(track_index)?
            .items
            .iter()
            .map(|item| (item.start, item.end))
            .collect(),
        _ => unreachable!(),
    };
    for (start, item_end) in times {
        if start <= snapped_time && snapped_time < item_end {
            return None;
        }
        if start > snapped_time {
            end = end.min(start);
            break;
        }
    }
    if end <= snapped_time {
        return None;
    }
    let item_index = match kind {
        TrackKind::Comment => crate::items::insert_sorted(
            &mut project.comment_tracks.get_mut(track_index)?.items,
            CommentItem::new(snapped_time, end, String::new()),
        ),
        TrackKind::Caption => crate::items::insert_sorted(
            &mut project.caption_tracks.get_mut(track_index)?.items,
            CaptionItem::new(snapped_time, end, String::new()),
        ),
        _ => unreachable!(),
    };
    let item_key = ItemKey {
        kind,
        track_index,
        item_index,
    };
    Some(item_key)
}
