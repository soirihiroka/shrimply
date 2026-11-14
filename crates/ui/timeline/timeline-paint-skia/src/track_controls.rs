use super::icons::*;
use crate::canvas::Stroke;
use crate::math::{row_screen_y, track_label_button_y};
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
    if height <= 0.0 || LABEL_WIDTH <= 0.0 {
        return;
    }

    painter.rect_filled(
        rect(0.0, 0.0, LABEL_WIDTH, height),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(0.0, RULER_HEIGHT, LABEL_WIDTH, 1.0),
        0,
        crate::theme::current().sidebar_border,
    );
    painter.rect_filled(
        rect(LABEL_WIDTH - 1.0, 0.0, 1.0, height),
        0,
        crate::theme::current().sidebar_border,
    );

    let icon_color = crate::theme::current().view_fg;
    let row_clip = rect(
        0.0,
        RULER_HEIGHT,
        LABEL_WIDTH,
        (height - RULER_HEIGHT).max(0.0),
    );
    let row_painter = painter.with_clip_rect(row_clip);
    for (row, track_row) in rows.iter().enumerate() {
        let y = row_screen_y(row, view);
        if y + TRACK_HEIGHT <= RULER_HEIGHT || y >= height {
            continue;
        }
        let y = row_screen_y(row, view);
        let Some(key) = track_row.root_key else {
            let kind = match &track_row.address {
                crate::project::TrackAddress::Comment { .. } => TrackKind::Comment,
                crate::project::TrackAddress::Caption { .. } => TrackKind::Caption,
                crate::project::TrackAddress::Video { .. } => TrackKind::Video,
                crate::project::TrackAddress::Audio { .. } => TrackKind::Audio,
            };
            draw_folded_track_label(&row_painter, y, kind, track_row.depth);
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
                width: LABEL_WIDTH,
                index_x: TRACK_INDEX_COLUMN_WIDTH / 2.0,
                text: &format!("{prefix}{}", key.track_index),
                accent: track_kind_label_color(key.kind),
                selected: selected_tracks.contains(&track_row.address),
                theme: &crate::theme::current(),
            },
        );

        let button_y = track_label_button_y(y);
        let toggle_rect = rect(
            TRACK_LABEL_TOGGLE_X,
            button_y,
            TRACK_LABEL_BUTTON_SIZE,
            TRACK_LABEL_BUTTON_SIZE,
        );
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

        let add_rect = rect(
            TRACK_LABEL_ADD_X,
            button_y,
            TRACK_LABEL_BUTTON_SIZE,
            TRACK_LABEL_BUTTON_SIZE,
        );
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

        let (record_action, record_active, record_icon) = match key.kind {
            TrackKind::Video => (
                TrackLabelAction::VideoRecord,
                track_control_draw.active_video_recording_key == Some(key),
                "screencast-recorded-symbolic",
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
        let record_rect = rect(
            TRACK_LABEL_RECORD_X,
            button_y,
            TRACK_LABEL_BUTTON_SIZE,
            TRACK_LABEL_BUTTON_SIZE,
        );
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
            LABEL_WIDTH,
            0.0,
            (TIMELINE_X - LABEL_WIDTH).max(0.0),
            height,
        ),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(TIMELINE_X - 1.0, 0.0, 1.0, height),
        0,
        crate::theme::current().sidebar_border,
    );
    painter.rect_filled(
        rect(0.0, 0.0, LABEL_WIDTH, RULER_HEIGHT),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(0.0, RULER_HEIGHT, LABEL_WIDTH, 1.0),
        0,
        crate::theme::current().sidebar_border,
    );
}

fn draw_folded_track_label(painter: &TimelinePainter, y: f64, kind: TrackKind, depth: usize) {
    const INDENT: f32 = 8.0;
    const BRANCH_LENGTH: f32 = 12.0;
    let branch_x = TRACK_INDEX_COLUMN_WIDTH as f32 * 0.5 + depth.saturating_sub(1) as f32 * INDENT;
    let center_y = (y + TRACK_HEIGHT * 0.5) as f32;
    painter.rect_filled(
        rect(0.0, y, LABEL_WIDTH, TRACK_HEIGHT - 1.0),
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
        rect(0.0, y + TRACK_HEIGHT - 1.0, LABEL_WIDTH, 1.0),
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
    let frame = controls.buttons.entry(id).or_default().draw(
        painter.canvas(),
        shrimply_components_skia::button::Config::new(rect, color)
            .style(shrimply_components_skia::button::Style::Flat)
            .padding(shrimply_components_skia::button::Padding::ImageButton)
            .checked(checked),
    );
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
    if !(0.0..TIMELINE_X).contains(&x) || y < RULER_HEIGHT {
        return None;
    }
    let content_y = y + view.scroll_y;
    let (kind, track_index, row) = rows::track_at_y(project, content_y)?;
    let row_y = row_screen_y(row, view);
    let button_y = track_label_button_y(row_y);
    let key = TrackKey { kind, track_index };
    if hit_track_label_button(x, y, TRACK_LABEL_TOGGLE_X, button_y) {
        Some((key, TrackLabelAction::Toggle))
    } else if hit_track_label_button(x, y, TRACK_LABEL_ADD_X, button_y) {
        Some((key, TrackLabelAction::Add))
    } else if hit_track_label_button(x, y, TRACK_LABEL_RECORD_X, button_y) {
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

fn hit_track_label_button(x: f64, y: f64, button_x: f64, button_y: f64) -> bool {
    shrimply_components_skia::button::Button::hit_test(
        crate::canvas::Rect::from_xywh(
            button_x as f32,
            button_y as f32,
            TRACK_LABEL_BUTTON_SIZE as f32,
            TRACK_LABEL_BUTTON_SIZE as f32,
        ),
        crate::canvas::vec2(x as f32, y as f32),
    )
}
