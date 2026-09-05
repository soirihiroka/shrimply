use super::*;

pub(super) use shrimply_skia_adw_core::icon::Icon;
pub(super) fn draw_track_toggle_icon(
    painter: &TimelinePainter,
    rect: Rect,
    kind: TrackKind,
    enabled: bool,
    color: Color,
) {
    let icon = match (kind, enabled) {
        (TrackKind::Video, false) => Icon("eye-not-looking-symbolic"),
        (TrackKind::Video, true) => Icon("eye-open-negative-filled-symbolic"),
        (TrackKind::Caption, false) => Icon("closed-captioning-off-symbolic"),
        (TrackKind::Caption, true) => Icon("closed-captioning-symbolic"),
        (TrackKind::Audio, false) => Icon("speaker-0-symbolic"),
        (TrackKind::Audio, true) => Icon("speaker-3-symbolic"),
    };
    draw_icon(painter, rect, icon, color);
}

pub(super) fn draw_icon(painter: &TimelinePainter, rect: Rect, icon: Icon, color: Color) {
    let size = rect.width().min(rect.height()).min(16.0);
    let icon_rect = Rect::from_center_size(rect.center(), vec2(size, size));
    shrimply_skia_adw_core::icon::draw(painter.canvas(), icon, icon_rect, color);
}

pub(super) fn draw_item_icon(painter: &TimelinePainter, item_rect: Rect, icon: Icon, color: Color) {
    const ICON_SIZE: f32 = 16.0;
    const ICON_PADDING: f32 = 6.0;
    if item_rect.width() < ICON_SIZE + ICON_PADDING * 2.0 {
        return;
    }
    let icon_rect = Rect::from_min_size(
        vec2(
            item_rect.min.x + ICON_PADDING,
            item_rect.center().y - ICON_SIZE / 2.0,
        ),
        vec2(ICON_SIZE, ICON_SIZE),
    );
    draw_icon(painter, icon_rect, icon, color);
}
