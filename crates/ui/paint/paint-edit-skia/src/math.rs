use glam::Vec2;
use shrimply_paint_geometry::{eraser_sweep_intervals, point_hits_eraser_sweep};
use shrimply_paint_model::PaintPoint;

const MINIMUM_ERASE_INTERVAL: f32 = 1.0e-6;
pub(super) fn erase_fragments(
    points: &[PaintPoint],
    start: Vec2,
    end: Vec2,
    radius: f32,
) -> Option<Vec<Vec<PaintPoint>>> {
    match points {
        [] => return None,
        [point] => {
            return point_hits_eraser_sweep(point.position, start, end, radius).then(Vec::new);
        }
        _ => {}
    }

    let polyline: Vec<_> = points.iter().map(|point| point.position).collect();
    let mut erased: Vec<_> = eraser_sweep_intervals(&polyline, start, end, radius)
        .into_iter()
        .map(|interval| {
            (
                interval.segment_index as f32 + interval.start,
                interval.segment_index as f32 + interval.end,
            )
        })
        .filter(|(start, end)| end - start > MINIMUM_ERASE_INTERVAL)
        .collect();
    if erased.is_empty() {
        return None;
    }

    erased.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut merged: Vec<(f32, f32)> = Vec::with_capacity(erased.len());
    for interval in erased {
        if let Some(previous) = merged.last_mut()
            && interval.0 <= previous.1 + MINIMUM_ERASE_INTERVAL
        {
            previous.1 = previous.1.max(interval.1);
        } else {
            merged.push(interval);
        }
    }

    let maximum = (points.len() - 1) as f32;
    let mut cursor = 0.0;
    let mut fragments = Vec::new();
    for (start, end) in merged {
        if start > cursor + MINIMUM_ERASE_INTERVAL {
            fragments.push(fragment(points, cursor, start));
        }
        cursor = cursor.max(end);
    }
    if maximum > cursor + MINIMUM_ERASE_INTERVAL {
        fragments.push(fragment(points, cursor, maximum));
    }
    Some(fragments)
}

fn fragment(points: &[PaintPoint], start: f32, end: f32) -> Vec<PaintPoint> {
    let mut fragment = Vec::new();
    push_distinct(&mut fragment, sample_at(points, start));

    let first_index = start.floor() as usize + 1;
    let end_index = end.ceil() as usize;
    for &point in &points[first_index..end_index] {
        push_distinct(&mut fragment, point);
    }
    push_distinct(&mut fragment, sample_at(points, end));
    fragment
}

fn sample_at(points: &[PaintPoint], position: f32) -> PaintPoint {
    let index = position.floor() as usize;
    let amount = position - index as f32;
    if amount == 0.0 || index + 1 == points.len() {
        return points[index];
    }

    PaintPoint {
        position: points[index]
            .position
            .lerp(points[index + 1].position, amount),
        pressure: match (points[index].pressure, points[index + 1].pressure) {
            (Some(start), Some(end)) => Some(start + (end - start) * amount),
            _ => None,
        },
    }
}

fn push_distinct(points: &mut Vec<PaintPoint>, point: PaintPoint) {
    if points
        .last()
        .is_none_or(|previous| previous.position != point.position)
    {
        points.push(point);
    }
}

pub(crate) fn rounded_tool_size(value: f32) -> f32 {
    if value < 1.0 {
        (value * 10.0).round() / 10.0
    } else {
        value.round()
    }
}
