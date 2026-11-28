use crate::canvas::Stroke;
use crate::math::{row_screen_y, track_button_bounds};
use crate::*;

pub fn draw_track_label_pane(
    painter: &TimelinePainter,
    project: &Project,
    rows: &[rows::TrackRow],
    selected_tracks: &[crate::project::TrackAddress],
    track_control_draw: &mut TrackControlDraw<'_>,
    view: impl Into<Viewport>,
    height: f64,
) {
    let view = view.into();
    let label_width = view.label_width();
    let controls_visible = view.track_controls_progress >= 1.0;
    if height <= 0.0 || label_width <= 0.0 {
        return;
    }

    painter.rect_filled(
        rect(0.0, 0.0, label_width, height),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(0.0, RULER_HEIGHT, label_width, 1.0),
        0,
        crate::theme::current().sidebar_border,
    );
    painter.rect_filled(
        rect(label_width - 1.0, 0.0, 1.0, height),
        0,
        crate::theme::current().sidebar_border,
    );

    let icon_color = crate::theme::current().view_fg;
    let row_clip = rect(
        0.0,
        RULER_HEIGHT,
        label_width,
        (height - RULER_HEIGHT).max(0.0),
    );
    let row_painter = painter.with_clip_rect(row_clip);
    for (row, track_row) in rows.iter().enumerate() {
        let y = row_screen_y(row, view);
        if y + TRACK_HEIGHT <= RULER_HEIGHT || y >= height {
            continue;
        }
        let Some(key) = track_row.root_key else {
            let kind = match &track_row.address {
                crate::project::TrackAddress::Comment { .. } => TrackKind::Comment,
                crate::project::TrackAddress::Caption { .. } => TrackKind::Caption,
                crate::project::TrackAddress::Video { .. } => TrackKind::Video,
                crate::project::TrackAddress::Audio { .. } => TrackKind::Audio,
            };
            draw_folded_track_label(&row_painter, y, kind, track_row.depth, label_width);
            continue;
        };

        let prefix = match key.kind {
            TrackKind::Comment => 'C',
            TrackKind::Caption => 'S',
            TrackKind::Video => 'V',
            TrackKind::Audio => 'A',
        };
        crate::draw_track_label(
            &row_painter,
            crate::TrackLabel {
                y,
                width: label_width,
                index_x: TRACK_INDEX_COLUMN_WIDTH / 2.0,
                text: &format!("{prefix}{}", key.track_index),
                accent: track_kind_label_color(key.kind),
                selected: selected_tracks.contains(&track_row.address),
                theme: &crate::theme::current(),
            },
        );

        let add_rect = track_button_bounds(view, row, TrackLabelAction::Add)
            .expect("visible track has an add button");
        let add_icon_rect = draw_track_icon_button(
            &row_painter,
            add_rect,
            icon_color,
            (key, TrackLabelAction::Add),
            false,
            track_control_draw,
        );
        draw_icon(
            &row_painter,
            add_icon_rect,
            Icon("plus-symbolic"),
            icon_color,
        );

        if !controls_visible {
            continue;
        }
        let toggle_rect = track_button_bounds(view, row, TrackLabelAction::Toggle)
            .expect("visible controls have a toggle button");
        let toggle_icon_rect = draw_track_icon_button(
            &row_painter,
            toggle_rect,
            icon_color,
            (key, TrackLabelAction::Toggle),
            track_enabled(project, key),
            track_control_draw,
        );
        draw_track_toggle_icon(
            &row_painter,
            toggle_icon_rect,
            key.kind,
            track_enabled(project, key),
            icon_color,
        );

        let (record_action, record_active, record_icon) = match key.kind {
            TrackKind::Video => (
                TrackLabelAction::VideoRecord,
                track_control_draw.active_video_recording_key == Some(key),
                "video-camera-symbolic",
            ),
            TrackKind::Audio => (
                TrackLabelAction::AudioRecord,
                track_control_draw.active_audio_recording_key == Some(key),
                "mic-1-symbolic",
            ),
            TrackKind::Comment => continue,
            TrackKind::Caption => continue,
        };
        let record_color = if record_active {
            Color::RED1
        } else {
            icon_color
        };
        let record_rect = track_button_bounds(view, row, record_action)
            .expect("recording track has a record button");
        let record_icon_rect = draw_track_icon_button(
            &row_painter,
            record_rect,
            record_color,
            (key, record_action),
            record_active,
            track_control_draw,
        );
        draw_icon(
            &row_painter,
            record_icon_rect,
            Icon(record_icon),
            record_color,
        );
    }

    painter.rect_filled(
        rect(
            label_width,
            0.0,
            (view.timeline_x() - label_width).max(0.0),
            height,
        ),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(view.timeline_x() - 1.0, 0.0, 1.0, height),
        0,
        crate::theme::current().sidebar_border,
    );
    painter.rect_filled(
        rect(0.0, 0.0, label_width, RULER_HEIGHT),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(0.0, RULER_HEIGHT, label_width, 1.0),
        0,
        crate::theme::current().sidebar_border,
    );
}

fn draw_folded_track_label(
    painter: &TimelinePainter,
    y: f64,
    kind: TrackKind,
    depth: usize,
    label_width: f64,
) {
    const INDENT: f32 = 8.0;
    const BRANCH_LENGTH: f32 = 12.0;
    let branch_x = TRACK_INDEX_COLUMN_WIDTH as f32 * 0.5 + depth.saturating_sub(1) as f32 * INDENT;
    let center_y = (y + TRACK_HEIGHT * 0.5) as f32;
    painter.rect_filled(
        rect(0.0, y, label_width, TRACK_HEIGHT - 1.0),
        0,
        crate::theme::current().sidebar_shade.alpha_multiply(0.22),
    );
    painter.line_segment(
        [vec2(branch_x, y as f32), vec2(branch_x, center_y)],
        Stroke::new(1.0, track_kind_label_color(kind)),
    );
    painter.line_segment(
        [
            vec2(branch_x, center_y),
            vec2(branch_x + BRANCH_LENGTH, center_y),
        ],
        Stroke::new(1.0, track_kind_label_color(kind)),
    );
    painter.rect_filled(
        rect(0.0, y + TRACK_HEIGHT - 1.0, label_width, 1.0),
        0,
        crate::theme::current().sidebar_shade,
    );
}

pub fn track_kind_label_color(kind: TrackKind) -> Color {
    if kind == TrackKind::Caption {
        Color::YELLOW5
    } else {
        rows::color(kind)
    }
}

fn draw_track_icon_button(
    painter: &TimelinePainter,
    rect: Rect,
    color: Color,
    id: TrackButtonId,
    checked: bool,
    controls: &mut TrackControlDraw<'_>,
) -> Rect {
    let config = shrimply_components_skia::button::Config::new(rect, color)
        .style(shrimply_components_skia::button::Style::Flat)
        .padding(shrimply_components_skia::button::Padding::ImageButton)
        .checked(checked);
    let config = if controls.touch {
        config.interaction(
            false,
            !controls.external_indication && controls.touch_pressed == Some(id),
        )
    } else {
        config
    };
    let frame = controls
        .buttons
        .entry(id)
        .or_default()
        .draw(painter.canvas(), config);
    *controls.animation_active |= frame.animating;
    frame.content_bounds
}

pub fn track_enabled(project: &Project, key: TrackKey) -> bool {
    match key.kind {
        TrackKind::Comment => project
            .comment_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
        TrackKind::Caption => project
            .caption_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
        TrackKind::Video => project
            .video_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
        TrackKind::Audio => project
            .audio_tracks
            .get(key.track_index)
            .is_some_and(|track| track.enabled),
    }
}

pub fn track_label_action_at(
    project: &Project,
    view: Viewport,
    x: f64,
    y: f64,
) -> Option<(TrackKey, TrackLabelAction)> {
    if !(0.0..view.timeline_x()).contains(&x) || y < RULER_HEIGHT {
        return None;
    }
    let content_y = y + view.scroll_y;
    let (kind, track_index, row) = rows::track_at_y(project, content_y)?;
    let key = TrackKey { kind, track_index };
    let hit = |action| {
        track_button_bounds(view, row, action)
            .is_some_and(|bounds| bounds.contains(crate::canvas::vec2(x as f32, y as f32)))
    };
    if hit(TrackLabelAction::Add) {
        return Some((key, TrackLabelAction::Add));
    }
    if view.track_controls_progress < 1.0 {
        return Some((key, TrackLabelAction::Select));
    }
    if hit(TrackLabelAction::Toggle) {
        Some((key, TrackLabelAction::Toggle))
    } else if hit(TrackLabelAction::AudioRecord) {
        match kind {
            TrackKind::Audio => Some((key, TrackLabelAction::AudioRecord)),
            TrackKind::Video => Some((key, TrackLabelAction::VideoRecord)),
            TrackKind::Comment => Some((key, TrackLabelAction::Select)),
            TrackKind::Caption => Some((key, TrackLabelAction::Select)),
        }
    } else {
        Some((key, TrackLabelAction::Select))
    }
}
