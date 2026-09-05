use super::*;
use crate::project::ItemRef;
use shrimply_i18n_core::{text, text_args};

pub fn draw_import_preview(
    draw: &TimelineDraw<'_>,
    project: &Project,
    import_preview: &TimelineImportPreview,
    virtual_tracks: &[(TrackKind, usize)],
) {
    let preview = &import_preview.preview;
    for stream_index in 0..preview.video_streams {
        let track_index = preview.video_base + stream_index;
        let Some(row) =
            preview_row_for_track(project, virtual_tracks, TrackKind::Video, track_index)
        else {
            continue;
        };
        let y = row_screen_y(row, draw.view);
        draw_video_item(
            draw.painter,
            NaturalEndMarker {
                start: preview.start,
                end: preview.end,
                position: None,
                repeat_interval: None,
                real_start: None,
                real_end: None,
            },
            match import_preview
                .visual_kind
                .expect("video import preview has no visual kind")
            {
                import::VisualMediaKind::Video => Icon("video-camera-symbolic"),
                import::VisualMediaKind::Image => Icon("image-symbolic"),
                import::VisualMediaKind::Gif => Icon("container3-symbolic"),
                import::VisualMediaKind::Svg => Icon("boxy-svg-symbolic"),
                import::VisualMediaKind::Pdf => Icon("image-symbolic"),
                import::VisualMediaKind::Manim => Icon("manim-symbolic"),
                import::VisualMediaKind::Blender => Icon("blender-symbolic"),
                import::VisualMediaKind::LayeredImage => Icon("image-symbolic"),
                import::VisualMediaKind::Obj | import::VisualMediaKind::Gaussian => {
                    Icon("3d-object-symbolic")
                }
            },
            draw.timeline_x,
            y,
            draw.view,
            false,
        );
        draw_item_drag_outline(
            draw.painter,
            preview.start,
            preview.end,
            draw.timeline_x,
            y,
            draw.view,
            true,
        );
    }

    for stream_index in 0..preview.audio_streams {
        let track_index = preview.audio_base + stream_index;
        let Some(row) =
            preview_row_for_track(project, virtual_tracks, TrackKind::Audio, track_index)
        else {
            continue;
        };
        let y = row_screen_y(row, draw.view);
        let item = crate::project::AudioItem::builder(preview.start, preview.end)
            .id(uuid::Uuid::nil())
            .source_duration(import_preview.duration)
            .track_id(stream_index as u32)
            .file(import_preview.source.clone())
            .build();
        draw_audio_item(draw, &item, item.start, y, false);
        draw_item_drag_outline(
            draw.painter,
            preview.start,
            preview.end,
            draw.timeline_x,
            y,
            draw.view,
            true,
        );
    }
}

pub fn draw_text_drop_preview(
    draw: &TimelineDraw<'_>,
    project: &Project,
    preview: &external_content::TextPreview,
) {
    let Some(row) = row_for_track(project, preview.kind, preview.track_index) else {
        return;
    };
    let y = row_screen_y(row, draw.view);
    match preview.kind {
        TrackKind::Caption => draw_caption_item(
            draw.painter,
            &CaptionItem::new(preview.start, preview.end, preview.text.clone()),
            draw.timeline_x,
            y,
            draw.view,
            false,
        ),
        TrackKind::Video => draw_video_item(
            draw.painter,
            NaturalEndMarker {
                start: preview.start,
                end: preview.end,
                position: None,
                repeat_interval: None,
                real_start: None,
                real_end: None,
            },
            Icon("draw-text-symbolic"),
            draw.timeline_x,
            y,
            draw.view,
            false,
        ),
        TrackKind::Audio => return,
    }
    draw_item_drag_outline(
        draw.painter,
        preview.start,
        preview.end,
        draw.timeline_x,
        y,
        draw.view,
        true,
    );
}

pub fn draw_dragged_group(
    draw: &TimelineDraw<'_>,
    project: &Project,
    group: &DraggedGroup,
    virtual_tracks: &[(TrackKind, usize)],
) {
    for item in &group.items {
        let Some((start, end)) = target_item_times(group, item) else {
            continue;
        };
        let Some(track_index) = target_track_index(group, item) else {
            continue;
        };
        let Some(row) = group
            .cross_scope_preview_row
            .or_else(|| preview_row_for_track(project, virtual_tracks, item.key.kind, track_index))
        else {
            continue;
        };
        let y = row_screen_y(row, draw.view);

        match item.key.kind {
            TrackKind::Caption => {
                if let Some(source) = project
                    .caption_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    let mut preview = source.clone();
                    preview.start = start;
                    preview.end = end;
                    draw_caption_item(draw.painter, &preview, draw.timeline_x, y, draw.view, false);
                }
            }
            TrackKind::Video => {
                if let Some(source) = project
                    .video_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    draw_video_item(
                        draw.painter,
                        preview_video_marker(source, start, end, PreviewTimeMode::Move),
                        video_item_icon(&source.content),
                        draw.timeline_x,
                        y,
                        draw.view,
                        false,
                    );
                }
            }
            TrackKind::Audio => {
                if let Some(source) = project
                    .audio_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    let preview = preview_audio_item(source, start, end, PreviewTimeMode::Move);
                    draw_audio_item(draw, &preview, preview.start, y, false);
                }
            }
        }

        draw_preview_item_transitions(
            draw,
            project,
            item.key,
            start,
            end,
            PreviewTimeMode::Move,
            y,
        );

        draw_item_drag_status_outline(
            draw.painter,
            (start, end),
            draw.timeline_x,
            y,
            draw.view,
            group.valid_drop,
            if group.cross_scope_preview_row.is_none()
                && matches!(
                    group.preview_status,
                    DragPreviewStatus::Blocked | DragPreviewStatus::Overwrite
                )
            {
                DragPreviewStatus::Clear
            } else {
                group.preview_status
            },
        );
    }

    for indicator in &group.overwrite_indicators {
        draw_drag_indicator(
            draw,
            project,
            virtual_tracks,
            *indicator,
            DragPreviewStatus::Overwrite,
        );
    }

    for indicator in &group.blocked_indicators {
        draw_drag_indicator(
            draw,
            project,
            virtual_tracks,
            *indicator,
            DragPreviewStatus::Blocked,
        );
    }
}

pub fn draw_folded_new_track_preview(
    draw: &TimelineDraw<'_>,
    project: &Project,
    drag: &folded_sequence::FoldedDrag,
) {
    let Some(row) = drag.cross_scope_preview_row else {
        return;
    };
    let Some(preview) = drag.items.iter().find(|item| item.key == drag.key) else {
        return;
    };
    let y = row_screen_y(row, draw.view);
    match project.item(&preview.key) {
        Some(ItemRef::Video(source)) => draw_video_item(
            draw.painter,
            preview_video_marker(
                source,
                preview.target_start,
                preview.target_end,
                PreviewTimeMode::Move,
            ),
            video_item_icon(&source.content),
            draw.timeline_x,
            y,
            draw.view,
            true,
        ),
        Some(ItemRef::Audio(source)) => {
            let item = preview_audio_item(
                source,
                preview.target_start,
                preview.target_end,
                PreviewTimeMode::Move,
            );
            draw_audio_item(draw, &item, item.start, y, true);
        }
        Some(ItemRef::Caption(_)) | None => return,
    }
    draw_item_drag_status_outline(
        draw.painter,
        (preview.target_start, preview.target_end),
        draw.timeline_x,
        y,
        draw.view,
        drag.valid_drop,
        drag.preview_status,
    );
}

pub fn draw_preview_item_transitions(
    draw: &TimelineDraw<'_>,
    project: &Project,
    key: ItemKey,
    start: Time,
    end: Time,
    mode: PreviewTimeMode,
    y: f64,
) {
    let Some(address) = crate::selection_state::item_address(project, key) else {
        return;
    };
    let Some((mut intro, mut outro)) = transition_durations(project, &address) else {
        return;
    };
    if matches!(mode, PreviewTimeMode::Resize) {
        (intro, outro) = fitted_transition_durations(end.saturating_sub(start), intro, outro);
    }
    let color = match key.kind {
        TrackKind::Video => Color::BLUE1,
        TrackKind::Audio => Color::GREEN1,
        TrackKind::Caption => return,
    };
    draw_item_transitions(
        draw.painter,
        &address,
        start,
        end,
        intro,
        outro,
        None,
        None,
        draw.timeline_x,
        y,
        draw.view,
        color,
    );
}

pub fn draw_drag_indicator(
    draw: &TimelineDraw<'_>,
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    indicator: DragIndicator,
    status: DragPreviewStatus,
) {
    let Some(row) = preview_row_for_track(
        project,
        virtual_tracks,
        indicator.kind,
        indicator.track_index,
    ) else {
        return;
    };
    let y = row_screen_y(row, draw.view);
    let (item_x, item_width) =
        item_rect(indicator.start, indicator.end, draw.timeline_x, draw.view);
    if item_width <= 0.0 || TRACK_HEIGHT <= 0.0 {
        return;
    }
    let (fill, stroke) = match status {
        DragPreviewStatus::Overwrite => (
            Color::new(1.0, 0.68, 0.20, 0.0),
            Color::new(1.0, 0.68, 0.20, 1.0),
        ),
        DragPreviewStatus::Blocked => (
            Color::new(1.0, 0.16, 0.12, 0.18),
            Color::new(1.0, 0.32, 0.28, 1.0),
        ),
        DragPreviewStatus::NewTrack | DragPreviewStatus::Clear => (
            Color::new(0.78, 0.88, 1.0, 0.12),
            Color::new(0.78, 0.88, 1.0, 1.0),
        ),
    };
    let indicator_rect = rect(
        item_x + 0.5,
        y + 0.5,
        (item_width - 1.0).max(1.0),
        TRACK_HEIGHT - 2.0,
    );
    draw.painter.rect_filled(indicator_rect, 0, fill);
    draw.painter.rect_stroke(
        indicator_rect,
        0,
        Stroke::new(2.0, stroke),
        StrokeKind::Inside,
    );
}

pub fn draw_cut_preview(
    painter: &TimelinePainter,
    project: &Project,
    view: TimelineViewState,
    cut: &TimelineCut,
    timeline_x: f64,
    timeline_width: f64,
) {
    let started = Instant::now();
    for key in &cut.keys {
        draw_cut_line(
            painter,
            project,
            view,
            key,
            cut.time.as_secs_f64(),
            timeline_x,
            timeline_width,
        );
    }
    let elapsed = started.elapsed();
    if elapsed.as_micros() > 500 {
        tracing::debug!(
            "timeline cut draw key={:?}:{} keys={} elapsed_us={}",
            cut.key.kind(),
            cut.key.item_id(),
            cut.keys.len(),
            elapsed.as_micros()
        );
    }
}

pub fn draw_cut_line(
    painter: &TimelinePainter,
    project: &Project,
    view: TimelineViewState,
    key: &crate::project::ItemAddress,
    time: f64,
    timeline_x: f64,
    timeline_width: f64,
) {
    let Some(row) = crate::items::row_for_address(project, &key.track()) else {
        return;
    };
    let start_y = row_screen_y(row, view);
    let cut_x = time_to_x(time, timeline_x, view);
    if cut_x < timeline_x || cut_x > timeline_x + timeline_width {
        return;
    }

    let dash = 6.0;
    let gap = 4.0;
    let mut y = start_y;
    let end_y = start_y + TRACK_HEIGHT;
    let stroke = Stroke::new(2.0, Color::new(0.94, 0.84, 0.30, 1.0));

    while y < end_y {
        let segment_end = (y + dash).min(end_y);
        painter.line_segment(
            [
                vec2(cut_x as f32, y as f32),
                vec2(cut_x as f32, segment_end as f32),
            ],
            stroke,
        );
        y += dash + gap;
    }
}

pub fn draw_virtual_track_ghosts(
    painter: &TimelinePainter,
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    timeline_x: f64,
    timeline_width: f64,
    height: f64,
    view: TimelineViewState,
) {
    for &virtual_track in virtual_tracks {
        let Some(row) = visual_row_for_virtual_track(project, virtual_tracks, virtual_track) else {
            continue;
        };
        let y = row_screen_y(row, view);
        if y >= height {
            continue;
        }

        let kind = virtual_track.0;
        let color = track_kind_label_color(kind);
        let ghost = rect(timeline_x, y, timeline_width, TRACK_HEIGHT.min(height - y));
        painter.rect_filled(ghost, 0, color.alpha_multiply(0.10));
        painter.rect_stroke(
            ghost,
            0,
            Stroke::new(1.0, color.alpha_multiply(0.72)),
            StrokeKind::Inside,
        );
        painter.system_text(
            vec2((timeline_x + 8.0) as f32, (y + 11.0) as f32),
            text_args("New %{kind}", &[("kind", text(kind.label()).into_owned())]),
            FontId::proportional(12.0),
            color.alpha_multiply(0.92),
        );
        draw_track_divider(painter, timeline_x, y, timeline_width);
    }
}

pub fn draw_resize_drag(
    draw: &TimelineDraw<'_>,
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    resize: &ResizeDrag,
) {
    for item in &resize.items {
        let Some((start, end)) = resize_item_times(resize, item) else {
            continue;
        };
        let Some(row) = row_for_track(project, item.key.kind, item.key.track_index) else {
            continue;
        };
        let y = row_screen_y(row, draw.view);
        match item.key.kind {
            TrackKind::Caption => {
                if let Some(source) = project
                    .caption_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    let mut preview = source.clone();
                    preview.start = start;
                    preview.end = end;
                    draw_caption_item(draw.painter, &preview, draw.timeline_x, y, draw.view, false);
                }
            }
            TrackKind::Video => {
                if let Some(source) = project
                    .video_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    draw_video_item(
                        draw.painter,
                        preview_video_marker(source, start, end, PreviewTimeMode::Resize),
                        video_item_icon(&source.content),
                        draw.timeline_x,
                        y,
                        draw.view,
                        false,
                    );
                }
            }
            TrackKind::Audio => {
                if let Some(source) = project
                    .audio_tracks
                    .get(item.key.track_index)
                    .and_then(|track| track.items.get(item.key.item_index))
                {
                    let preview = preview_audio_item(source, start, end, PreviewTimeMode::Resize);
                    draw_audio_item(draw, &preview, source.start, y, false);
                }
            }
        }
        draw_preview_item_transitions(
            draw,
            project,
            item.key,
            start,
            end,
            PreviewTimeMode::Resize,
            y,
        );
        draw_item_drag_status_outline(
            draw.painter,
            (start, end),
            draw.timeline_x,
            y,
            draw.view,
            resize.valid,
            resize.preview_status,
        );
    }
    for indicator in &resize.overwrite_indicators {
        draw_drag_indicator(
            draw,
            project,
            virtual_tracks,
            *indicator,
            DragPreviewStatus::Overwrite,
        );
    }
    for indicator in &resize.blocked_indicators {
        draw_drag_indicator(
            draw,
            project,
            virtual_tracks,
            *indicator,
            DragPreviewStatus::Blocked,
        );
    }
}

pub fn preview_row_for_track(
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    kind: TrackKind,
    track_index: usize,
) -> Option<usize> {
    let virtual_track = (kind, track_index);
    if virtual_tracks.contains(&virtual_track) {
        return visual_row_for_virtual_track(project, virtual_tracks, virtual_track);
    }
    visual_row_for_track(project, kind, track_index, virtual_tracks)
}

pub fn active_virtual_tracks(
    dragged_group: Option<&DraggedGroup>,
    import_preview: Option<&TimelineImportPreview>,
) -> Vec<(TrackKind, usize)> {
    let mut tracks = Vec::new();
    if let Some(group) = dragged_group {
        tracks.extend(group.new_tracks.iter().copied());
    } else if let Some(preview) = import_preview {
        tracks.extend(preview.preview.virtual_tracks.iter().copied());
    }
    tracks.sort_by_key(|track| (kind_order(track.0), track.1));
    tracks.dedup();
    tracks
}

pub fn visual_row_for_track(
    project: &Project,
    kind: TrackKind,
    track_index: usize,
    virtual_tracks: &[(TrackKind, usize)],
) -> Option<usize> {
    crate::items::projected_row_for_track(project, kind, track_index, virtual_tracks)
}

pub fn visual_row_for_virtual_track(
    project: &Project,
    virtual_tracks: &[(TrackKind, usize)],
    virtual_track: (TrackKind, usize),
) -> Option<usize> {
    crate::items::projected_row_for_virtual_track(project, virtual_tracks, virtual_track)
}

pub fn kind_order(kind: TrackKind) -> usize {
    match kind {
        TrackKind::Caption => 0,
        TrackKind::Video => 1,
        TrackKind::Audio => 2,
    }
}

pub fn draw_item_drag_outline(
    painter: &TimelinePainter,
    start: Time,
    end: Time,
    x: f64,
    y: f64,
    view: TimelineViewState,
    valid_drop: bool,
) {
    draw_item_drag_status_outline(
        painter,
        (start, end),
        x,
        y,
        view,
        valid_drop,
        if valid_drop {
            DragPreviewStatus::Clear
        } else {
            DragPreviewStatus::Blocked
        },
    );
}

pub fn draw_item_drag_status_outline(
    painter: &TimelinePainter,
    range: (Time, Time),
    x: f64,
    y: f64,
    view: TimelineViewState,
    valid_drop: bool,
    status: DragPreviewStatus,
) {
    let (start, end) = range;
    let (item_x, item_width) = item_rect(start, end, x, view);
    if item_width <= 0.0 || TRACK_HEIGHT <= 0.0 {
        return;
    }
    let color = match (valid_drop, status) {
        (false, _) | (_, DragPreviewStatus::Blocked) => Color::new(1.0, 0.32, 0.28, 1.0),
        (_, DragPreviewStatus::Overwrite) => Color::new(1.0, 0.68, 0.20, 1.0),
        (_, DragPreviewStatus::NewTrack | DragPreviewStatus::Clear) => {
            Color::new(0.78, 0.88, 1.0, 1.0)
        }
    };
    painter.rect_stroke(
        rect(
            item_x + 0.5,
            y + 0.5,
            (item_width - 1.0).max(1.0),
            TRACK_HEIGHT - 2.0,
        ),
        0,
        Stroke::new(2.0, color),
        StrokeKind::Inside,
    );
}
