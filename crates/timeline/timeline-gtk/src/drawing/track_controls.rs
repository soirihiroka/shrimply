use super::icons::*;
use super::*;

pub(super) fn draw_track_label_pane(
    painter: &TimelinePainter,
    project: &Project,
    selected_tracks: &[crate::project::TrackAddress],
    track_control_draw: &mut TrackControlDraw<'_>,
    view: TimelineViewState,
    height: f64,
) {
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
    for_each_visible_track_row(project, view, height, |track_row, row| {
        let y = row_screen_y(row, view);
        let Some(key) = track_row.root_key else {
            let kind = match &track_row.address {
                crate::project::TrackAddress::Caption { .. } => TrackKind::Caption,
                crate::project::TrackAddress::Video { .. } => TrackKind::Video,
                crate::project::TrackAddress::Audio { .. } => TrackKind::Audio,
            };
            draw_folded_track_label(&row_painter, y, kind, track_row.depth);
            return;
        };

        if selected_tracks.contains(&track_row.address) {
            row_painter.rect_filled(
                rect(0.0, y, LABEL_WIDTH, TRACK_HEIGHT - 1.0),
                0,
                crate::theme::current()
                    .view_fg
                    .alpha_multiply(TRACK_SELECTION_LABEL_ALPHA),
            );
        }

        row_painter.rect_filled(
            rect(0.0, y, 3.0, (TRACK_HEIGHT - 1.0).max(1.0)),
            0,
            track_kind_label_color(key.kind),
        );
        row_painter.rect_filled(
            rect(0.0, y + TRACK_HEIGHT - 1.0, LABEL_WIDTH, 1.0),
            0,
            crate::theme::current().sidebar_shade,
        );

        let prefix = match key.kind {
            TrackKind::Caption => 'S',
            TrackKind::Video => 'V',
            TrackKind::Audio => 'A',
        };
        row_painter.text(
            vec2(
                (TRACK_INDEX_COLUMN_WIDTH / 2.0) as f32,
                (y + TRACK_HEIGHT / 2.0) as f32,
            ),
            Align2::CENTER_CENTER,
            format!("{prefix}{}", key.track_index),
            FontId::proportional(12.0),
            track_kind_label_color(key.kind),
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
            TrackKind::Caption => return,
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
    });

    painter.rect_filled(
        rect(
            LABEL_WIDTH,
            0.0,
            (timeline_x() - LABEL_WIDTH).max(0.0),
            height,
        ),
        0,
        crate::theme::current().view_bg,
    );
    painter.rect_filled(
        rect(timeline_x() - 1.0, 0.0, 1.0, height),
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

pub(super) fn track_kind_label_color(kind: TrackKind) -> Color {
    if kind == TrackKind::Caption {
        Color::YELLOW5
    } else {
        items::track_color(kind)
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
        shrimply_skia_adw_core::button::Config::new(rect, color)
            .style(shrimply_skia_adw_core::button::Style::Flat)
            .padding(shrimply_skia_adw_core::button::Padding::ImageButton)
            .checked(checked),
    );
    *controls.animation_active |= frame.animating;
    frame.content_bounds
}
