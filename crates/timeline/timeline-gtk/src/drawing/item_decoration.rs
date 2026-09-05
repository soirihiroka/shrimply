use super::super::*;

use super::*;

#[allow(clippy::too_many_arguments)]
pub(in crate::drawing) fn draw_item_transitions(
    painter: &TimelinePainter,
    key: &crate::project::ItemAddress,
    start: Time,
    end: Time,
    mut intro: Option<Time>,
    mut outro: Option<Time>,
    focused: Option<&(crate::project::ItemAddress, TransitionSide)>,
    drag: Option<&TransitionDrag>,
    x: f64,
    y: f64,
    view: TimelineViewState,
    color: Color,
) {
    if let Some(drag) = drag.filter(|drag| &drag.key == key) {
        let value = (!drag.remove).then_some(drag.target_timeline_duration);
        match drag.side {
            TransitionSide::Intro => intro = value,
            TransitionSide::Outro => outro = value,
        }
    }
    let start_x = x + (start.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x = x + (end.as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    for (side, duration) in [
        (TransitionSide::Intro, intro),
        (TransitionSide::Outro, outro),
    ] {
        let Some(duration) = duration.filter(|duration| *duration > Time::ZERO) else {
            continue;
        };
        let boundary_x = match side {
            TransitionSide::Intro => {
                x + (start.saturating_add(duration).as_secs_f64() - view.scroll_seconds)
                    / view.seconds_per_pixel
            }
            TransitionSide::Outro => {
                x + (end.saturating_sub(duration).as_secs_f64() - view.scroll_seconds)
                    / view.seconds_per_pixel
            }
        };
        let mut path = skia_safe::PathBuilder::new();
        match side {
            TransitionSide::Intro => {
                path.move_to((start_x as f32, y as f32));
                path.line_to((start_x as f32, (y + TRACK_HEIGHT) as f32));
                path.line_to((boundary_x as f32, y as f32));
            }
            TransitionSide::Outro => {
                path.move_to((boundary_x as f32, y as f32));
                path.line_to((end_x as f32, y as f32));
                path.line_to((end_x as f32, (y + TRACK_HEIGHT) as f32));
            }
        }
        path.close();
        let path = path.detach();
        painter.path_filled(&path, color.alpha_multiply(0.24));
        let selected =
            focused.is_some_and(|(focused, focused_side)| focused == key && *focused_side == side);
        painter.path_stroke(
            &path,
            Stroke::new(if selected { 2.0 } else { 1.0 }, color.alpha_multiply(0.9)),
        );
        if selected {
            painter.rect_filled(rect(boundary_x - 3.0, y, 6.0, 7.0), 1, color);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::drawing) fn draw_clip_transition(
    painter: &TimelinePainter,
    key: &crate::project::ItemAddress,
    cut: Time,
    duration: Time,
    focused: Option<&(crate::project::ItemAddress, TransitionSide)>,
    drag_duration: Option<Option<Time>>,
    x: f64,
    y: f64,
    view: TimelineViewState,
    color: Color,
) {
    let duration = match drag_duration {
        Some(drag_duration) => match drag_duration {
            Some(duration) => duration,
            None => return,
        },
        None => duration,
    };
    let half = crate::math::clip_transition_half_duration(duration);
    let start_x =
        x + (cut.saturating_sub(half).as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let end_x =
        x + (cut.saturating_add(half).as_secs_f64() - view.scroll_seconds) / view.seconds_per_pixel;
    let transition_rect = rect(start_x, y, (end_x - start_x).max(1.0), TRACK_HEIGHT);
    painter.rect_filled(transition_rect, 0, color.alpha_multiply(0.3));
    let selected =
        focused.is_some_and(|(focused, side)| focused == key && *side == TransitionSide::Outro);
    if selected {
        painter.rect_stroke(
            transition_rect,
            0,
            Stroke::new(2.0, color.alpha_multiply(0.9)),
            StrokeKind::Inside,
        );
        painter.rect_filled(rect(start_x - 3.0, y, 6.0, 7.0), 1, color);
        painter.rect_filled(rect(end_x - 3.0, y, 6.0, 7.0), 1, color);
    }
}

pub(crate) fn row_y(row: usize) -> f64 {
    RULER_HEIGHT + row as f64 * TRACK_HEIGHT
}

pub(crate) fn row_screen_y(row: usize, view: TimelineViewState) -> f64 {
    row_y(row) - view.scroll_y
}

pub(in crate::drawing) fn draw_selected_track_fill(
    painter: &TimelinePainter,
    selected_tracks: &[crate::project::TrackAddress],
    address: &crate::project::TrackAddress,
    x: f64,
    y: f64,
    width: f64,
) {
    if selected_tracks.contains(address) {
        let foreground = crate::theme::current().view_fg;
        let fill = foreground.alpha_multiply(TRACK_SELECTION_ROW_ALPHA);
        let edge = foreground.alpha_multiply(TRACK_SELECTION_EDGE_ALPHA);
        let height = (TRACK_HEIGHT - 1.0).max(1.0);
        painter.rect_filled(rect(x, y, width, height), 0, fill);
        painter.rect_filled(rect(x, y, width, 1.0), 0, edge);
        painter.rect_filled(rect(x, y + height - 1.0, width, 1.0), 0, edge);
    }
}

pub(in crate::drawing) fn draw_track_divider(
    painter: &TimelinePainter,
    x: f64,
    y: f64,
    width: f64,
) {
    if width <= 0.0 || TRACK_HEIGHT <= 1.0 {
        return;
    }
    painter.rect_filled(
        rect(x, y + TRACK_HEIGHT - 1.0, width, 1.0),
        0,
        crate::theme::current().sidebar_shade,
    );
}

pub(in crate::drawing) fn add_tiny_item(
    columns: &mut [bool],
    edges: &mut [bool],
    timeline_x: f64,
    item_x: f64,
    item_width: f64,
) {
    add_tiny_item_fill(columns, timeline_x, item_x, item_width);
    for edge_x in [item_x, item_x + item_width] {
        let edge = ((edge_x - timeline_x).round().max(0.0) as usize).min(edges.len() - 1);
        edges[edge] = true;
    }
}

pub(in crate::drawing) fn add_tiny_item_fill(
    columns: &mut [bool],
    timeline_x: f64,
    item_x: f64,
    item_width: f64,
) {
    let first = ((item_x - timeline_x).floor().max(0.0) as usize).min(columns.len());
    let end = ((item_x + item_width - timeline_x).ceil().max(0.0) as usize).min(columns.len());
    columns[first..end].fill(true);
}

pub(in crate::drawing) fn draw_tiny_item_fill(
    painter: &TimelinePainter,
    columns: &[bool],
    timeline_x: f64,
    y: f64,
    color: Color,
) {
    let mut segments = Vec::new();
    let mut column = 0;
    while column < columns.len() {
        if !columns[column] {
            column += 1;
            continue;
        }
        let start = column;
        while column < columns.len() && columns[column] {
            column += 1;
        }
        let center_y = (y + TRACK_HEIGHT * 0.5) as f32;
        segments.extend([
            vec2((timeline_x + start as f64) as f32, center_y),
            vec2((timeline_x + column as f64) as f32, center_y),
        ]);
    }
    painter.line_segments(&segments, Stroke::new(TRACK_HEIGHT as f32, color));
}

pub(in crate::drawing) fn draw_tiny_item_edges(
    painter: &TimelinePainter,
    edges: &[bool],
    timeline_x: f64,
    y: f64,
    width: f32,
    color: Color,
) {
    let mut segments = Vec::new();
    for (column, edge) in edges.iter().copied().enumerate() {
        if !edge {
            continue;
        }
        let x = (timeline_x + column as f64) as f32;
        segments.extend([vec2(x, y as f32), vec2(x, (y + TRACK_HEIGHT) as f32)]);
    }
    painter.line_segments(&segments, Stroke::new(width, color));
}

pub(in crate::drawing) fn draw_tiny_item_horizontal_edges(
    painter: &TimelinePainter,
    columns: &[bool],
    timeline_x: f64,
    y: f64,
    color: Color,
) {
    let mut segments = Vec::new();
    let mut column = 0;
    while column < columns.len() {
        if !columns[column] {
            column += 1;
            continue;
        }
        let start = column;
        while column < columns.len() && columns[column] {
            column += 1;
        }
        let left = (timeline_x + start as f64) as f32;
        let right = (timeline_x + column as f64) as f32;
        let inset = ITEM_BORDER_STROKE_WIDTH * 0.5;
        segments.extend([
            vec2(left, y as f32 + inset),
            vec2(right, y as f32 + inset),
            vec2(left, (y + TRACK_HEIGHT) as f32 - inset),
            vec2(right, (y + TRACK_HEIGHT) as f32 - inset),
        ]);
    }
    painter.line_segments(&segments, Stroke::new(ITEM_BORDER_STROKE_WIDTH, color));
}

pub(in crate::drawing) fn draw_natural_end_marker(
    painter: &TimelinePainter,
    marker: NaturalEndMarker,
    x: f64,
    y: f64,
    view: TimelineViewState,
    color: Color,
) {
    let Some(first_position) = marker.position else {
        return;
    };
    let mut position = first_position;
    if let Some(interval) = marker.repeat_interval {
        if interval <= Time::ZERO {
            return;
        }
        while position > marker.start {
            let previous = Time {
                seconds: position.seconds - interval.seconds,
            };
            if previous <= marker.start {
                break;
            }
            position = previous;
        }
        while position <= marker.start {
            position = position.saturating_add(interval);
        }
    } else if position <= marker.start {
        return;
    }
    while position < marker.end {
        let marker_x = time_to_x(position.as_secs_f64(), x, view);
        painter.rect_filled(
            rect(marker_x - 1.0, y + 3.0, 2.0, (TRACK_HEIGHT - 6.0).max(1.0)),
            0,
            color.alpha_multiply(0.9),
        );
        let Some(interval) = marker.repeat_interval else {
            break;
        };
        position = position.saturating_add(interval);
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::drawing) fn draw_item_box(
    painter: &TimelinePainter,
    item_rect: Rect,
    fill: Color,
    selected: bool,
    selected_border_color: Color,
) {
    if item_rect.width() <= 0.0 || item_rect.height() <= 0.0 {
        return;
    }

    draw_item_fill(painter, item_rect, fill);
    draw_item_border(painter, item_rect, selected, selected_border_color);
}

pub(in crate::drawing) fn draw_timed_item_box(painter: &TimelinePainter, item: TimedItemBox) {
    if item.bounds.width() <= 0.0 || item.bounds.height() <= 0.0 {
        return;
    }

    let real_span = marker_real_span(item.marker);
    draw_item_fill(
        painter,
        item.bounds,
        if real_span.is_some() {
            item.fill.alpha_multiply(0.45)
        } else {
            item.fill
        },
    );
    if let Some((real_start, real_end)) = real_span {
        let (real_x, real_width) = item_rect(real_start, real_end, item.timeline_x, item.view);
        draw_item_fill(
            painter,
            rect(
                real_x,
                f64::from(item.bounds.min.y),
                real_width,
                f64::from(item.bounds.height()),
            ),
            item.fill,
        );
    }
    draw_item_border(
        painter,
        item.bounds,
        item.selected,
        item.selected_border_color,
    );
}

pub(in crate::drawing) fn marker_real_span(marker: NaturalEndMarker) -> Option<(Time, Time)> {
    let real_start = marker.real_start?;
    let real_end = marker.real_end?;
    let start = real_start.min(real_end).max(marker.start);
    let end = real_start.max(real_end).min(marker.end);
    (start < end).then_some((start, end))
}

pub(in crate::drawing) fn draw_item_fill(painter: &TimelinePainter, item_rect: Rect, fill: Color) {
    painter.rect_filled(item_rect, 0, fill);
}

pub(in crate::drawing) fn draw_item_border(
    painter: &TimelinePainter,
    item_rect: Rect,
    selected: bool,
    selected_border_color: Color,
) {
    if item_rect.width() <= 0.0 || item_rect.height() <= 0.0 {
        return;
    }

    let border_color = if selected {
        selected_border_color
    } else {
        crate::theme::current().sidebar_border
    };

    let border = f64::from(ITEM_BORDER_STROKE_WIDTH)
        .min(f64::from(item_rect.width()) * 0.5)
        .min(f64::from(item_rect.height()) * 0.5);
    painter.rect_filled(
        rect(
            f64::from(item_rect.min.x),
            f64::from(item_rect.min.y),
            border,
            f64::from(item_rect.height()),
        ),
        0,
        border_color,
    );
    painter.rect_filled(
        rect(
            f64::from(item_rect.max.x) - border,
            f64::from(item_rect.min.y),
            border,
            f64::from(item_rect.height()),
        ),
        0,
        border_color,
    );
    painter.rect_filled(
        rect(
            f64::from(item_rect.min.x) + border,
            f64::from(item_rect.min.y),
            (f64::from(item_rect.width()) - border * 2.0).max(0.0),
            border,
        ),
        0,
        border_color,
    );
    painter.rect_filled(
        rect(
            f64::from(item_rect.min.x) + border,
            f64::from(item_rect.max.y) - border,
            (f64::from(item_rect.width()) - border * 2.0).max(0.0),
            border,
        ),
        0,
        border_color,
    );
}
