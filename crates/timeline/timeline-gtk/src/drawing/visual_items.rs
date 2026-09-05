use super::super::*;
use super::audio::preview_media_time_offset;
use super::decoration::*;

pub(in crate::drawing) fn draw_video_item(
    painter: &TimelinePainter,
    marker: NaturalEndMarker,
    icon: Icon,
    x: f64,
    y: f64,
    view: TimelineViewState,
    selected: bool,
) {
    let (clip_x, clip_width) = item_rect(marker.start, marker.end, x, view);
    draw_timed_item_box(
        painter,
        TimedItemBox {
            marker,
            bounds: rect(clip_x, y, clip_width, TRACK_HEIGHT),
            fill: Color::BLUE5,
            timeline_x: x,
            view,
            selected,
            selected_border_color: Color::BLUE1,
        },
    );
    draw_item_icon(
        painter,
        rect(clip_x, y, clip_width, TRACK_HEIGHT),
        icon,
        Color::BLUE1,
    );
    draw_natural_end_marker(painter, marker, x, y, view, Color::BLUE1);
}

pub(in crate::drawing) fn video_item_icon(content: &VideoItemContent) -> Icon {
    match content {
        VideoItemContent::Gif => Icon("container3-symbolic"),
        VideoItemContent::Media => Icon("video-camera-symbolic"),
        VideoItemContent::Image => Icon("image-symbolic"),
        VideoItemContent::Text(_) => Icon("draw-text-symbolic"),
        VideoItemContent::Shape(_) => Icon("shapes-large-symbolic"),
        VideoItemContent::Paint(_) => Icon("applications-graphics-symbolic"),
        VideoItemContent::Background(_) => Icon("preferences-desktop-wallpaper-symbolic"),
        VideoItemContent::Obj(_) => Icon("3d-object-symbolic"),
        VideoItemContent::Gaussian(_) => Icon("3d-object-symbolic"),
        VideoItemContent::Svg => Icon("boxy-svg-symbolic"),
        VideoItemContent::Pdf(_) => Icon("image-symbolic"),
        VideoItemContent::Manim(_) => Icon("manim-symbolic"),
        VideoItemContent::Blender(_) => Icon("blender-symbolic"),
        VideoItemContent::LayeredImage(_) => Icon("image-symbolic"),
        VideoItemContent::FoldedSequence(_) => Icon("folder-symbolic"),
    }
}

pub(in crate::drawing) fn draw_caption_item(
    painter: &TimelinePainter,
    item: &CaptionItem,
    x: f64,
    y: f64,
    view: TimelineViewState,
    selected: bool,
) {
    let (clip_x, clip_width) = item_rect(item.start, item.end, x, view);
    let accent_color = Color::YELLOW5;
    draw_item_box(
        painter,
        rect(clip_x, y, clip_width, TRACK_HEIGHT),
        accent_color,
        selected,
        Color::YELLOW1,
    );
    if !item.text.is_empty() {
        let font_id = FontId::proportional(10.0);
        let color = crate::theme::current().view_fg;
        let max_width = (clip_width - ITEM_PADDING_X * 2.0 - 4.0).max(0.0) as f32;
        painter
            .with_clip_rect(rect(
                clip_x + ITEM_PADDING_X,
                y,
                (clip_width - ITEM_PADDING_X * 2.0).max(1.0),
                TRACK_HEIGHT,
            ))
            .system_text_ellipsized(
                vec2((clip_x + ITEM_PADDING_X + 2.0) as f32, (y + 12.0) as f32),
                &item.text,
                font_id,
                color,
                max_width,
            );
    }
}

pub(in crate::drawing) fn video_natural_end_marker(
    item: &crate::project::VideoItem,
) -> Option<Time> {
    if item.repeats_keyframes() {
        generated_item_natural_end_position(item)
    } else {
        media_item_natural_end_position(
            item.start,
            item.time_offset,
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )
    }
}

pub(in crate::drawing) fn video_real_start(item: &crate::project::VideoItem) -> Option<Time> {
    if item.repeats_keyframes() {
        generated_item_natural_span(item).map(|(start, _)| start)
    } else {
        media_real_span(
            item.start,
            item.time_offset,
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )
        .map(|(start, _)| start)
    }
}

pub(in crate::drawing) fn video_real_end(item: &crate::project::VideoItem) -> Option<Time> {
    if item.repeats_keyframes() {
        let (_, end) = generated_item_keyframe_span(item)?;
        Some(item.start.saturating_add(Time {
            seconds: end.seconds - item.animation_time_offset.seconds,
        }))
    } else {
        media_real_span(
            item.start,
            item.time_offset,
            item.source_duration,
            item.playback_speed,
            item.repeat_strategy,
        )
        .map(|(_, end)| end)
    }
}

pub(in crate::drawing) fn preview_video_marker(
    source: &crate::project::VideoItem,
    start: Time,
    end: Time,
    mode: PreviewTimeMode,
) -> NaturalEndMarker {
    let mut preview = source.clone();
    preview.start = start;
    preview.end = end;
    if matches!(mode, PreviewTimeMode::Resize) {
        preview.time_offset = preview_media_time_offset(
            source.time_offset,
            source.start,
            start,
            source.playback_speed,
            source.repeat_strategy,
            source.source_duration,
        );
        preview.animation_time_offset = source
            .animation_time_offset
            .saturating_add(start.signed_sub(source.start));
    }
    NaturalEndMarker {
        start,
        end,
        position: video_natural_end_marker(&preview),
        repeat_interval: video_natural_end_interval(&preview),
        real_start: video_real_start(&preview),
        real_end: video_real_end(&preview),
    }
}
