use super::*;

pub fn draw(input: TrackDrawInput<'_>, first_visible_row: usize, last_visible_row: usize) {
    let TrackDrawInput {
        painter,
        project,
        selected_items,
        selected_tracks,
        dragged_group,
        resize_drag,
        virtual_tracks,
        view,
        timeline_x,
        timeline_width,
        ..
    } = input;
    for (track_index, track) in project.comment_tracks.iter().enumerate() {
        let Some(row) =
            visual_row_for_track(project, TrackKind::Comment, track_index, virtual_tracks)
        else {
            continue;
        };
        if row < first_visible_row || row >= last_visible_row {
            continue;
        }
        let y = row_screen_y(row, view);
        draw_selected_track_fill(
            painter,
            selected_tracks,
            &crate::project::TrackAddress::Comment { track_id: track.id },
            timeline_x,
            y,
            timeline_width,
        );
        for (index, item) in timeline_search::overlapping_disjoint(
            &track.items,
            input.visible_times.0,
            input.visible_times.1,
        ) {
            let (item_x, item_width) = item_rect(item.start, item.end, timeline_x, view);
            if item_x + item_width <= timeline_x || item_x >= timeline_x + timeline_width {
                continue;
            }
            let key = ItemKey {
                kind: TrackKind::Comment,
                track_index,
                item_index: index,
            };
            let dragged = is_item_dragged(dragged_group, key.kind, track_index, index);
            let resizing =
                resize_drag.is_some_and(|resize| resize.items.iter().any(|item| item.key == key));
            if !dragged && !resizing {
                draw_comment_item(
                    painter,
                    item,
                    timeline_x,
                    y,
                    view,
                    is_item_selected(selected_items, key.kind, track_index, index),
                );
            }
        }
        draw_track_divider(painter, timeline_x, y, timeline_width);
    }
}
