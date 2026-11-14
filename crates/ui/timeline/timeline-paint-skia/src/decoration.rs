use super::*;

pub fn draw_item_box(
    painter: &TimelinePainter,
    item_rect: Rect,
    fill: Color,
    selected: bool,
    selected_border_color: Color,
    border_color: Color,
) {
    if item_rect.width() <= 0.0 || item_rect.height() <= 0.0 {
        return;
    }

    draw_item_fill(painter, item_rect, fill);
    draw_item_border(
        painter,
        item_rect,
        selected,
        selected_border_color,
        border_color,
    );
}

pub fn draw_item_fill(painter: &TimelinePainter, item_rect: Rect, fill: Color) {
    painter.rect_filled(item_rect, 0, fill);
}

pub fn draw_item_border(
    painter: &TimelinePainter,
    item_rect: Rect,
    selected: bool,
    selected_border_color: Color,
    border_color: Color,
) {
    if item_rect.width() <= 0.0 || item_rect.height() <= 0.0 {
        return;
    }

    let border_color = if selected {
        selected_border_color
    } else {
        border_color
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
