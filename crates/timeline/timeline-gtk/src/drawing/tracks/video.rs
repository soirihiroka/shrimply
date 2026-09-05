use super::*;

pub(super) fn draw(
    draw: &TimelineDraw<'_>,
    input: TrackDrawInput<'_>,
    content_height: f64,
    first_visible_row: usize,
    last_visible_row: usize,
) {
    let TrackDrawInput {
        painter,
        project,
        selected_items,
        selected_tracks,
        dragged_group,
        folded_drag,
        resize_drag,
        transition_drag,
        clip_transition_drag,
        focused_transition,
        live_video_recording,
        virtual_tracks,
        view,
        timeline_x,
        timeline_width,
        ..
    } = input;
    for (track_index, track) in project.video_tracks.iter().enumerate() {
        let Some(row) =
            visual_row_for_track(project, TrackKind::Video, track_index, virtual_tracks)
        else {
            continue;
        };
        draw_expanded_video_tracks(draw, input, track_index, row, content_height);
        if row < first_visible_row || row >= last_visible_row {
            continue;
        }
        let y = row_screen_y(row, view);
        draw_selected_track_fill(
            painter,
            selected_tracks,
            &crate::project::TrackAddress::Video {
                sequence_path: Vec::new(),
                track_id: track.id,
            },
            timeline_x,
            y,
            timeline_width,
        );
        let mut tiny_item_columns = vec![false; timeline_width.ceil() as usize];
        let mut tiny_item_outline_columns = vec![false; tiny_item_columns.len()];
        let mut tiny_selected_outline_columns = vec![false; tiny_item_columns.len()];
        let mut tiny_item_edges = vec![false; tiny_item_columns.len() + 1];
        let mut tiny_selected_edges = vec![false; tiny_item_columns.len() + 1];
        let mut selected_subpixel_columns = vec![false; tiny_item_columns.len()];
        let mut clip_transitions = Vec::new();
        for (index, item) in track.items.iter().enumerate() {
            let (item_x, item_width) = item_rect(item.start, item.end, timeline_x, view);
            let item_pixel_width =
                (item.end.as_secs_f64() - item.start.as_secs_f64()) / view.seconds_per_pixel;
            if item_x + item_width <= timeline_x || item_x >= timeline_x + timeline_width {
                continue;
            }
            let key = ItemKey {
                kind: TrackKind::Video,
                track_index,
                item_index: index,
            };
            let address = crate::project::ItemAddress::Video {
                sequence_path: Vec::new(),
                track_id: track.id,
                item_id: item.id,
            };
            let dragged = is_item_dragged(dragged_group, key.kind, track_index, index);
            let resizing =
                resize_drag.is_some_and(|resize| resize.items.iter().any(|item| item.key == key));
            let selected = is_item_selected(selected_items, key.kind, track_index, index);
            if !dragged && !resizing {
                if item_width >= MIN_DETAILED_ITEM_WIDTH {
                    let sequence_expanded =
                        matches!(item.content, VideoItemContent::FoldedSequence(_))
                            && folded_sequence::expanded(project, &[item.id]);
                    draw_video_item(
                        painter,
                        NaturalEndMarker {
                            start: item.start,
                            end: item.end,
                            position: video_natural_end_marker(item),
                            repeat_interval: video_natural_end_interval(item),
                            real_start: video_real_start(item),
                            real_end: video_real_end(item),
                        },
                        if sequence_expanded {
                            Icon("folder-open-symbolic")
                        } else {
                            video_item_icon(&item.content)
                        },
                        timeline_x,
                        y,
                        view,
                        selected,
                    );
                    if !matches!(
                        item.content,
                        VideoItemContent::Obj(_) | VideoItemContent::Gaussian(_)
                    ) {
                        draw_item_transitions(
                            painter,
                            &address,
                            item.start,
                            item.end,
                            item.transitions.intro.as_ref().map(|value| value.duration),
                            item.transitions.outro.as_ref().map(|value| value.duration),
                            focused_transition,
                            transition_drag,
                            timeline_x,
                            y,
                            view,
                            Color::BLUE1,
                        );
                    }
                    if let Some(transition) = item.transitions.to_next.as_ref() {
                        clip_transitions.push((address, item.end, transition.duration));
                    }
                } else if selected && item_pixel_width < SUBPIXEL_ITEM_WIDTH {
                    add_tiny_item_fill(
                        &mut selected_subpixel_columns,
                        timeline_x,
                        item_x,
                        item_width,
                    );
                } else {
                    add_tiny_item(
                        &mut tiny_item_columns,
                        if selected {
                            &mut tiny_selected_edges
                        } else {
                            &mut tiny_item_edges
                        },
                        timeline_x,
                        item_x,
                        item_width,
                    );
                    add_tiny_item_fill(
                        if selected {
                            &mut tiny_selected_outline_columns
                        } else {
                            &mut tiny_item_outline_columns
                        },
                        timeline_x,
                        item_x,
                        item_width,
                    );
                }
            }
        }
        draw_tiny_item_fill(painter, &tiny_item_columns, timeline_x, y, Color::BLUE5);
        draw_tiny_item_edges(
            painter,
            &tiny_item_edges,
            timeline_x,
            y,
            ITEM_BORDER_STROKE_WIDTH,
            crate::theme::current().sidebar_border,
        );
        draw_tiny_item_horizontal_edges(
            painter,
            &tiny_item_outline_columns,
            timeline_x,
            y,
            crate::theme::current().sidebar_border,
        );
        draw_tiny_item_edges(
            painter,
            &tiny_selected_edges,
            timeline_x,
            y,
            ITEM_BORDER_STROKE_WIDTH,
            Color::BLUE1,
        );
        draw_tiny_item_horizontal_edges(
            painter,
            &tiny_selected_outline_columns,
            timeline_x,
            y,
            Color::BLUE1,
        );
        draw_tiny_item_fill(
            painter,
            &selected_subpixel_columns,
            timeline_x,
            y,
            Color::BLUE1,
        );
        for (address, cut, duration) in clip_transitions {
            draw_clip_transition(
                painter,
                &address,
                cut,
                duration,
                focused_transition,
                clip_transition_drag
                    .filter(|drag| drag.outgoing == address)
                    .map(|drag| drag.target_duration),
                timeline_x,
                y,
                view,
                Color::BLUE1,
            );
        }
        let target = crate::project::TrackAddress::Video {
            sequence_path: Vec::new(),
            track_id: track.id,
        };
        if let Some(drag) = folded_drag {
            for preview in drag
                .items
                .iter()
                .filter(|item| item.target_track == target && item.key.track() != target)
            {
                let Some(source) = project.video_item(&preview.key) else {
                    continue;
                };
                draw_video_item(
                    painter,
                    preview_video_marker(
                        source,
                        preview.target_start,
                        preview.target_end,
                        PreviewTimeMode::Move,
                    ),
                    video_item_icon(&source.content),
                    timeline_x,
                    y,
                    view,
                    true,
                );
                draw_item_drag_status_outline(
                    painter,
                    (preview.target_start, preview.target_end),
                    timeline_x,
                    y,
                    view,
                    drag.valid_drop,
                    drag.preview_status,
                );
            }
        }
        if let Some(recording) = live_video_recording.filter(|recording| {
            recording.key.kind == TrackKind::Video && recording.key.track_index == track_index
        }) {
            draw_video_item(
                painter,
                NaturalEndMarker {
                    start: recording.start,
                    end: recording.end,
                    position: None,
                    repeat_interval: None,
                    real_start: None,
                    real_end: None,
                },
                Icon("video-camera-symbolic"),
                timeline_x,
                y,
                view,
                false,
            );
        }
        draw_track_divider(painter, timeline_x, y, timeline_width);
    }
}
