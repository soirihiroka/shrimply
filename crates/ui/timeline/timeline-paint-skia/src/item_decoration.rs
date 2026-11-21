use crate::*;
use decoration::draw_item_fill;

pub fn draw_natural_end_marker(
    painter: &TimelinePainter,
    marker: NaturalEndMarker,
    x: f64,
    y: f64,
    view: impl Into<Viewport>,
    color: Color,
) {
    let view = view.into();
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
        let marker_x = view.scale.time_to_x(position, x);
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

pub fn draw_timed_item_box(painter: &TimelinePainter, item: TimedItemBox) {
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
    decoration::draw_item_border(
        painter,
        item.bounds,
        item.selected,
        item.selected_border_color,
        theme::current().sidebar_border,
    );
}

pub fn marker_real_span(marker: NaturalEndMarker) -> Option<(Time, Time)> {
    let real_start = marker.real_start?;
    let real_end = marker.real_end?;
    let start = real_start.min(real_end).max(marker.start);
    let end = real_start.max(real_end).min(marker.end);
    (start < end).then_some((start, end))
}
