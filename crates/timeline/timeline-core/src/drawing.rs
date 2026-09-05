use super::*;

#[path = "drawing/items.rs"]
mod clip_items;
mod folded;
mod icons;
mod previews;
mod track_controls;
mod tracks;

use clip_items::*;
pub use clip_items::{item_rect, row_screen_y, row_y};
use folded::{draw_expanded_audio_tracks, draw_expanded_video_tracks};
use icons::*;
pub use previews::active_virtual_tracks;
use previews::*;
use track_controls::{draw_track_label_pane, track_kind_label_color};
use tracks::draw_tracks;

pub struct TimelineInput<'a, 'b> {
    pub painter: &'a TimelinePainter,
    pub project: &'a Project,
    pub playback_performance: &'a playback_performance::Snapshot,
    pub current_time: Time,
    pub waveforms: &'a WaveformMap,
    pub beats: &'a BeatMap,
    pub beat_grid_enabled: bool,
    pub selected_items: &'a [ItemKey],
    pub selected_nested_items: &'a [crate::project::ItemAddress],
    pub selected_tracks: &'a [crate::project::TrackAddress],
    pub selected_gap: Option<TrackGap>,
    pub track_control_draw: &'a mut TrackControlDraw<'b>,
    pub dragged_group: Option<&'a DraggedGroup>,
    pub folded_drag: Option<&'a folded_sequence::FoldedDrag>,
    pub resize_drag: Option<&'a ResizeDrag>,
    pub transition_drag: Option<&'a TransitionDrag>,
    pub clip_transition_drag: Option<&'a ClipTransitionDrag>,
    pub focused_transition: Option<(crate::project::ItemAddress, TransitionSide)>,
    pub import_preview: Option<&'a TimelineImportPreview>,
    pub text_drop_preview: Option<&'a external_content::TextPreview>,
    pub cut_preview: Option<&'a TimelineCut>,
    pub live_recording: Option<&'a LiveRecordingDraw>,
    pub live_video_recording: Option<LiveVideoRecordingDraw>,
    pub view: TimelineViewState,
    pub virtual_tracks: &'a [(TrackKind, usize)],
    pub width: f64,
    pub height: f64,
    pub timeline_width: f64,
    pub frame_step_seconds: f64,
    pub animation_seconds: f64,
    pub waveform_chunks_per_second: u32,
    pub accent_color: Color,
    pub overscroll: Option<(TimelineOverscrollEdge, f64)>,
    pub horizontal_scrollbar: Option<shrimply_skia_adw_core::Scrollbar>,
    pub vertical_scrollbar: Option<shrimply_skia_adw_core::Scrollbar>,
    pub software_cursor: Option<&'a TimelineSoftwareCursor>,
}

pub fn draw_timeline(input: TimelineInput<'_, '_>) {
    let TimelineInput {
        painter,
        project,
        playback_performance,
        current_time,
        waveforms,
        beats,
        beat_grid_enabled,
        selected_items,
        selected_nested_items,
        selected_tracks,
        selected_gap,
        track_control_draw,
        dragged_group,
        folded_drag,
        resize_drag,
        transition_drag,
        clip_transition_drag,
        focused_transition,
        import_preview,
        text_drop_preview,
        cut_preview,
        live_recording,
        live_video_recording,
        view,
        virtual_tracks,
        width,
        height,
        timeline_width,
        frame_step_seconds,
        animation_seconds,
        waveform_chunks_per_second,
        accent_color,
        overscroll,
        horizontal_scrollbar,
        vertical_scrollbar,
        software_cursor,
    } = input;
    let timeline_x = timeline_x();
    let content_height = height.max(0.0);
    let visible_track_height = (height - RULER_HEIGHT).max(0.0);
    let track_clip = rect(
        timeline_x,
        RULER_HEIGHT,
        timeline_width,
        visible_track_height,
    );

    painter.rect_filled(
        rect(0.0, 0.0, width, height),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(0.0, RULER_HEIGHT, width, 1.0),
        0,
        crate::theme::current().sidebar_border,
    );

    let timeline_empty = project
        .caption_tracks
        .iter()
        .all(|track| track.items.is_empty())
        && project
            .video_tracks
            .iter()
            .all(|track| track.items.is_empty())
        && project
            .audio_tracks
            .iter()
            .all(|track| track.items.is_empty());
    ruler::draw(
        painter,
        ruler::RulerDraw {
            scale: ruler_scale(view),
            timeline_x,
            timeline_width,
            content_height,
            frame_step_seconds,
            frame_rate: project.fps,
            hide_zero_label: timeline_empty,
            style: ruler::RulerStyle {
                height: RULER_HEIGHT,
                frame_tick_min_width: FRAME_TICK_MIN_WIDTH,
                grid_color: crate::theme::current().sidebar_shade,
                label_color: crate::theme::current()
                    .view_fg
                    .alpha_multiply(RULER_LABEL_ALPHA),
            },
        },
    );
    draw_performance_ranges(
        painter,
        playback_performance,
        view,
        timeline_x,
        timeline_width,
    );
    draw_track_label_pane(
        painter,
        project,
        selected_tracks,
        track_control_draw,
        view,
        content_height,
    );

    {
        let track_painter = painter.with_clip_rect(track_clip);
        let draw = TimelineDraw {
            painter: &track_painter,
            waveforms,
            timeline_x,
            timeline_width,
            waveform_chunks_per_second,
            view,
            animation_seconds,
        };

        draw_tracks(
            &track_painter,
            project,
            waveforms,
            selected_items,
            selected_nested_items,
            folded_drag,
            selected_tracks,
            dragged_group,
            resize_drag,
            transition_drag,
            clip_transition_drag,
            focused_transition.as_ref(),
            live_recording,
            live_video_recording,
            virtual_tracks,
            view,
            timeline_x,
            timeline_width,
            content_height,
            animation_seconds,
            waveform_chunks_per_second,
        );
        if let Some(gap) = selected_gap {
            draw_selected_gap(
                &track_painter,
                project,
                gap,
                timeline_x,
                timeline_width,
                view,
            );
        }

        if beat_grid_enabled {
            beat_grid::draw(
                &track_painter,
                project,
                beats,
                view,
                timeline_width,
                content_height,
            );
        }

        if let Some(selection) = view.selection {
            draw_selection(
                &track_painter,
                selection,
                timeline_x,
                timeline_width,
                content_height,
                view,
            );
        }
        if let Some(preview) = import_preview {
            draw_import_preview(&draw, project, preview, virtual_tracks);
        }
        if let Some(preview) = text_drop_preview {
            draw_text_drop_preview(&draw, project, preview);
        }
        if let Some(group) = dragged_group {
            draw_dragged_group(&draw, project, group, virtual_tracks);
        }
        if let Some(drag) = folded_drag {
            draw_folded_new_track_preview(&draw, project, drag);
        }
        if !virtual_tracks.is_empty() {
            draw_virtual_track_ghosts(
                &track_painter,
                project,
                virtual_tracks,
                timeline_x,
                timeline_width,
                content_height,
                view,
            );
        }
        if let Some(resize) = resize_drag {
            draw_resize_drag(&draw, project, virtual_tracks, resize);
        }
        if let Some(cut) = cut_preview {
            draw_cut_preview(draw.painter, project, view, cut, timeline_x, timeline_width);
        }
    }

    let playhead_x = time_to_x(current_time.as_secs_f64(), timeline_x, view);
    let playhead_clip = rect(timeline_x, 0.0, timeline_width, content_height);
    {
        let playhead_painter = painter.with_clip_rect(playhead_clip);
        draw_playhead(
            &playhead_painter,
            playhead_x,
            frame_width(view, frame_step_seconds),
            content_height,
            accent_color,
        );
    }
    if let Some((edge, distance)) = overscroll {
        draw_timeline_overscroll(
            painter,
            timeline_x,
            timeline_width,
            content_height,
            edge,
            distance,
        );
    }
    draw_vertical_slider(painter, vertical_scrollbar);
    draw_timeline_slider(painter, horizontal_scrollbar);
    if let Some(software_cursor) = software_cursor {
        software_cursor
            .cursor
            .draw(painter.canvas(), software_cursor.position);
    }
}

fn draw_performance_ranges(
    painter: &TimelinePainter,
    snapshot: &playback_performance::Snapshot,
    view: TimelineViewState,
    timeline_x: f64,
    timeline_width: f64,
) {
    let painter = painter.with_clip_rect(rect(timeline_x, 0.0, timeline_width, RULER_HEIGHT));
    let y = RULER_HEIGHT - PERFORMANCE_MARKER_HEIGHT;
    for range in &snapshot.visual_ranges {
        draw_performance_range(
            &painter,
            range.start,
            range.end,
            view,
            timeline_x,
            y,
            crate::theme::current()
                .view_fg
                .alpha_multiply(PERFORMANCE_VISUAL_ALPHA),
        );
    }
    for (level, color) in [
        (playback_performance::PerformanceLevel::Fast, Color::GREEN3),
        (playback_performance::PerformanceLevel::Low, Color::YELLOW3),
        (playback_performance::PerformanceLevel::Slow, Color::RED3),
    ] {
        for range in snapshot
            .performance_ranges
            .iter()
            .filter(|range| range.level == level)
        {
            draw_performance_range(&painter, range.start, range.end, view, timeline_x, y, color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_performance_range(
    painter: &TimelinePainter,
    start: Time,
    end: Time,
    view: TimelineViewState,
    timeline_x: f64,
    y: f64,
    color: Color,
) {
    let left = time_to_x(start.as_secs_f64(), timeline_x, view);
    let right = time_to_x(end.as_secs_f64(), timeline_x, view);
    if right > left {
        painter.rect_filled(
            rect(left, y, (right - left).max(1.0), PERFORMANCE_MARKER_HEIGHT),
            0,
            color,
        );
    }
}

fn draw_selected_gap(
    painter: &TimelinePainter,
    project: &Project,
    gap: TrackGap,
    timeline_x: f64,
    timeline_width: f64,
    view: TimelineViewState,
) {
    let Some(row) = row_for_track(project, gap.track.kind, gap.track.track_index) else {
        return;
    };
    let left = time_to_x(gap.start.as_secs_f64(), timeline_x, view).max(timeline_x);
    let right = time_to_x(gap.end.as_secs_f64(), timeline_x, view).min(timeline_x + timeline_width);
    if right <= left {
        return;
    }
    let selected = rect(
        left,
        row_screen_y(row, view),
        right - left,
        (TRACK_HEIGHT - 1.0).max(1.0),
    );
    let color = crate::items::track_color(gap.track.kind);
    painter.rect_filled(selected, 0, color.alpha_multiply(0.18));
    painter.rect_stroke(
        selected,
        0,
        Stroke::new(2.0, color.alpha_multiply(0.9)),
        StrokeKind::Inside,
    );
}

fn draw_timeline_overscroll(
    painter: &TimelinePainter,
    timeline_x: f64,
    timeline_width: f64,
    content_height: f64,
    edge: TimelineOverscrollEdge,
    distance: f64,
) {
    if timeline_width <= 0.0 || content_height <= RULER_HEIGHT {
        return;
    }

    shrimply_skia_adw_core::draw_overshoot(
        painter.canvas(),
        shrimply_skia_adw_core::Rect::from_xywh(
            timeline_x as f32,
            RULER_HEIGHT as f32,
            timeline_width as f32,
            (content_height - RULER_HEIGHT) as f32,
        ),
        edge,
        distance,
        Color::<f32>::WHITE,
    );
}

fn draw_selection(
    painter: &TimelinePainter,
    selection: TimelineSelection,
    timeline_x: f64,
    timeline_width: f64,
    height: f64,
    view: TimelineViewState,
) {
    let x1 = time_to_x(selection.start.as_secs_f64(), timeline_x, view);
    let x2 = time_to_x(selection.end.as_secs_f64(), timeline_x, view);
    let left = x1.min(x2).max(timeline_x);
    let right = x1.max(x2).min(timeline_x + timeline_width);
    let top = (selection.start_y.min(selection.end_y) - view.scroll_y).clamp(RULER_HEIGHT, height);
    let bottom =
        (selection.start_y.max(selection.end_y) - view.scroll_y).clamp(RULER_HEIGHT, height);
    if right <= left || bottom <= top {
        return;
    }

    let rect = rect(left, top, right - left, bottom - top);
    painter.rect_filled(rect, 0, Color::new(0.38, 0.62, 0.98, 0.18));
    painter.rect_stroke(
        rect,
        0,
        Stroke::new(1.0, Color::new(0.50, 0.72, 1.0, 0.78)),
        StrokeKind::Inside,
    );
}

fn draw_vertical_slider(
    painter: &TimelinePainter,
    scrollbar: Option<shrimply_skia_adw_core::Scrollbar>,
) {
    let Some(scrollbar) = scrollbar else { return };
    shrimply_skia_adw_core::slider::draw(painter.canvas(), scrollbar);
}

fn draw_timeline_slider(
    painter: &TimelinePainter,
    scrollbar: Option<shrimply_skia_adw_core::Scrollbar>,
) {
    let Some(scrollbar) = scrollbar else { return };
    shrimply_skia_adw_core::slider::draw(painter.canvas(), scrollbar);
}

fn draw_playhead(
    painter: &TimelinePainter,
    playhead_x: f64,
    frame_width: f64,
    height: f64,
    color: Color,
) {
    cursor::draw_playhead(
        painter,
        playhead_x,
        frame_width,
        height,
        color,
        cursor::PlayheadStyle {
            ruler_height: RULER_HEIGHT,
            frame_y: Some(RULER_HEIGHT - 4.0),
            handle_width: PLAYHEAD_HANDLE_WIDTH,
            handle_height: PLAYHEAD_HANDLE_HEIGHT,
            handle_top: PLAYHEAD_HANDLE_TOP,
            triangle_height: PLAYHEAD_HANDLE_TRIANGLE_HEIGHT,
        },
    );
}
