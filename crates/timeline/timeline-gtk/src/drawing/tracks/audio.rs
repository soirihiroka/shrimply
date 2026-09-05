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
        live_recording,
        virtual_tracks,
        view,
        timeline_x,
        timeline_width,
        ..
    } = input;
    for (track_index, track) in project.audio_tracks.iter().enumerate() {
        let Some(row) =
            visual_row_for_track(project, TrackKind::Audio, track_index, virtual_tracks)
        else {
            continue;
        };
        draw_expanded_audio_tracks(draw, input, track_index, row, content_height);
        if row < first_visible_row || row >= last_visible_row {
            continue;
        }
        let y = row_screen_y(row, view);
        draw_selected_track_fill(
            painter,
            selected_tracks,
            &crate::project::TrackAddress::Audio {
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
        let mut waveform_columns = vec![0.0; timeline_width.ceil() as usize];
        let mut clip_transitions = Vec::new();
        for (index, item) in track.items.iter().enumerate() {
            let (item_x, item_width) = item_rect(item.start, item.end, timeline_x, view);
            let item_pixel_width =
                (item.end.as_secs_f64() - item.start.as_secs_f64()) / view.seconds_per_pixel;
            if item_x + item_width <= timeline_x || item_x >= timeline_x + timeline_width {
                continue;
            }
            let key = ItemKey {
                kind: TrackKind::Audio,
                track_index,
                item_index: index,
            };
            let address = crate::project::ItemAddress::Audio {
                sequence_path: Vec::new(),
                track_id: track.id,
                item_id: item.id,
            };
            let dragged = is_item_dragged(dragged_group, key.kind, track_index, index);
            let resizing =
                resize_drag.is_some_and(|resize| resize.items.iter().any(|item| item.key == key));
            let selected = is_item_selected(selected_items, key.kind, track_index, index);
            if !dragged && !resizing {
                let detailed = item_width >= MIN_DETAILED_ITEM_WIDTH;
                let selected_subpixel = selected && item_pixel_width < SUBPIXEL_ITEM_WIDTH;
                if selected_subpixel {
                    add_tiny_item_fill(
                        &mut selected_subpixel_columns,
                        timeline_x,
                        item_x,
                        item_width,
                    );
                } else if !detailed {
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
                if !selected_subpixel {
                    if matches!(&item.source, crate::project::AudioSource::FoldedSequence(_)) {
                        let sequence_expanded = folded_sequence::expanded(project, &[item.id]);
                        draw_audio_item_with_waveform(
                            draw,
                            item,
                            item.start,
                            y,
                            WaveformState::Loaded(None),
                            Color::GREEN5.alpha_multiply(0.55),
                            Color::GREEN1,
                            selected,
                            Color::GREEN1,
                            detailed,
                            &mut waveform_columns,
                        );
                        draw_item_icon(
                            painter,
                            rect(item_x, y, item_width, TRACK_HEIGHT),
                            if sequence_expanded {
                                Icon("folder-open-symbolic")
                            } else {
                                Icon("folder-symbolic")
                            },
                            Color::GREEN1,
                        );
                    } else if matches!(&item.source, crate::project::AudioSource::Tts(_))
                        && item.file.as_os_str().is_empty()
                    {
                        draw_audio_item_with_waveform(
                            draw,
                            item,
                            item.start,
                            y,
                            WaveformState::Loaded(None),
                            Color::GREEN5.alpha_multiply(0.55),
                            Color::GREEN1,
                            selected,
                            Color::GREEN1,
                            detailed,
                            &mut waveform_columns,
                        );
                    } else {
                        draw_audio_item_into(
                            draw,
                            item,
                            item.start,
                            y,
                            selected,
                            detailed,
                            &mut waveform_columns,
                        );
                    }
                    if matches!(&item.source, crate::project::AudioSource::Tts(_)) {
                        draw_item_icon(
                            painter,
                            rect(item_x, y, item_width, TRACK_HEIGHT),
                            Icon("font-x-generic-symbolic"),
                            Color::GREEN1,
                        );
                    } else if matches!(&item.source, crate::project::AudioSource::Generator(_)) {
                        draw_item_icon(
                            painter,
                            rect(item_x, y, item_width, TRACK_HEIGHT),
                            Icon("sound-symbolic"),
                            Color::GREEN1,
                        );
                    }
                }
                if detailed && !selected_subpixel {
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
                        Color::GREEN1,
                    );
                    if let Some(transition) = item.transitions.to_next.as_ref() {
                        clip_transitions.push((address, item.end, transition.duration));
                    }
                }
            }
        }
        draw_tiny_item_fill(
            painter,
            &tiny_item_columns,
            timeline_x,
            y,
            Color::GREEN5.alpha_multiply(0.55),
        );
        draw_waveform_columns(painter, &waveform_columns, timeline_x, y, Color::GREEN1);
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
            Color::GREEN1,
        );
        draw_tiny_item_horizontal_edges(
            painter,
            &tiny_selected_outline_columns,
            timeline_x,
            y,
            Color::GREEN1,
        );
        draw_tiny_item_fill(
            painter,
            &selected_subpixel_columns,
            timeline_x,
            y,
            Color::GREEN1,
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
                Color::GREEN1,
            );
        }
        let target = crate::project::TrackAddress::Audio {
            sequence_path: Vec::new(),
            track_id: track.id,
        };
        if let Some(drag) = folded_drag {
            for preview in drag
                .items
                .iter()
                .filter(|item| item.target_track == target && item.key.track() != target)
            {
                let Some(source) = project.audio_item(&preview.key) else {
                    continue;
                };
                let preview_item = preview_audio_item(
                    source,
                    preview.target_start,
                    preview.target_end,
                    PreviewTimeMode::Move,
                );
                draw_audio_item(draw, &preview_item, preview_item.start, y, true);
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
        if let Some(recording) = live_recording
            && recording.key.kind == TrackKind::Audio
            && recording.key.track_index == track_index
        {
            draw_live_recording_item(draw, recording, y);
        }
        draw_track_divider(painter, timeline_x, y, timeline_width);
    }
}
