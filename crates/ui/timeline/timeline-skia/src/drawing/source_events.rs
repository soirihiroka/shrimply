use super::*;

pub(super) fn draw(
    draw: &TimelineDraw<'_>,
    project: &Project,
    item: &project::VideoItem,
    address: &project::ItemAddress,
    y: f64,
) {
    let Some(project::ItemRef::Video(source)) = project.item(address) else {
        return;
    };
    let Some(events) =
        shrimply_project_document::source_events::for_item(source, project.canvas_size)
    else {
        return;
    };
    let (from, to) = crate::math::visible_item_times(draw.view, draw.timeline_width);
    let mut cursor = from.max(item.start);
    let end = to.min(item.end);
    let pixel = Time {
        seconds: shrimply_math_core::fraction_from_f64(draw.view.seconds_per_pixel),
    };
    if pixel <= Time::ZERO {
        return;
    }
    let timing = project::GeneratedEventTiming::from(item);
    while cursor < end {
        let Some(next) =
            project::generated_event_neighbors(timing, &events.index, events.duration, cursor)[1]
        else {
            break;
        };
        if next > end {
            break;
        }
        if next > item.start && next < item.end {
            let x = crate::math::time_to_x(next.as_secs_f64(), draw.timeline_x, draw.view).floor()
                + 0.5;
            draw.painter.line_segment(
                [
                    vec2(x as f32, (y + 2.0) as f32),
                    vec2(x as f32, (y + TRACK_HEIGHT - 2.0) as f32),
                ],
                Stroke::new(1.0, Color::BLUE1.alpha_multiply(0.65)),
            );
        }
        // Pixel coalescing bounds drawing work even for millions of repeated events.
        cursor = next.saturating_add(pixel);
    }
}
