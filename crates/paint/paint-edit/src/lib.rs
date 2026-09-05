mod math;

use glam::{Mat3, Vec2};
use shrimply_paint_geometry::PreparedGeometry;
use shrimply_paint_model::{PaintDrawing, PaintFill, PaintPoint, PaintStroke};
use uuid::Uuid;

pub use preview::{
    DEFAULT_PAINT_ERASER_SCALE, PAINT_PREVIEW_FACET, PAINT_PREVIEW_STATE, PaintOnionFrame,
    PaintPointSelection, PaintPreviewMode, PaintPreviewRender, PaintPreviewState,
    ResolvedShakyPath, preview_provider, resolve_onion_frame,
};
pub use shrimply_paint_geometry::ResolvedPathOffset;

mod preview;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppendResult {
    pub changed: bool,
    pub stroke_id: Option<Uuid>,
}

/// Appends one ordered input batch to an active stroke, or starts a new stroke.
///
/// The returned id should be passed back as `active_stroke` for later batches in
/// the same gesture. A nonempty batch targeting a missing active stroke is a
/// host-state error and deliberately panics.
pub fn append_samples(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    active_stroke: Option<Uuid>,
    samples: &[PaintPoint],
    width_scale: f32,
    color_index: usize,
) -> AppendResult {
    let samples: Vec<_> = samples.iter().copied().filter_map(normalize).collect();
    if samples.is_empty() {
        return AppendResult {
            changed: false,
            stroke_id: active_stroke,
        };
    }

    let stroke_id = if let Some(stroke_id) = active_stroke {
        let stroke = drawing
            .strokes
            .iter_mut()
            .find(|stroke| stroke.id == stroke_id)
            .expect("active paint stroke is missing");
        let changed = append_distinct(&mut stroke.points, &samples);
        if changed {
            bump_revision(revision);
        }
        return AppendResult {
            changed,
            stroke_id: Some(stroke_id),
        };
    } else {
        if !width_scale.is_finite() || width_scale < 0.0 {
            return AppendResult {
                changed: false,
                stroke_id: None,
            };
        }
        let mut points = Vec::new();
        append_distinct(&mut points, &samples);
        let stroke = PaintStroke::new(points, width_scale, color_index);
        let stroke_id = stroke.id;
        drawing.strokes.push(stroke);
        stroke_id
    };

    bump_revision(revision);
    AppendResult {
        changed: true,
        stroke_id: Some(stroke_id),
    }
}

/// Erases every raw centerline interval covered by a swept circular eraser.
pub fn erase_sweep(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    start: Vec2,
    end: Vec2,
    radius: f32,
) -> bool {
    if !start.is_finite() || !end.is_finite() || !radius.is_finite() || radius < 0.0 {
        return false;
    }

    let mut changed = false;
    let mut strokes = Vec::with_capacity(drawing.strokes.len());
    for stroke in std::mem::take(&mut drawing.strokes) {
        let Some(fragments) = math::erase_fragments(&stroke.points, start, end, radius) else {
            strokes.push(stroke);
            continue;
        };

        changed = true;
        for (index, points) in fragments.into_iter().enumerate() {
            strokes.push(PaintStroke {
                id: if index == 0 {
                    stroke.id
                } else {
                    Uuid::new_v4()
                },
                correspondence_id: stroke.correspondence_id,
                width_scale: stroke.width_scale,
                color_index: stroke.color_index,
                points,
            });
        }
    }
    drawing.strokes = strokes;

    if changed {
        bump_revision(revision);
    }
    changed
}

/// Moves one stored raw sample without changing its pressure.
pub fn move_sample(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    stroke_id: Uuid,
    sample_index: usize,
    position: Vec2,
) -> bool {
    if !position.is_finite() {
        return false;
    }
    let Some(point) = drawing
        .strokes
        .iter_mut()
        .find(|stroke| stroke.id == stroke_id)
        .and_then(|stroke| stroke.points.get_mut(sample_index))
    else {
        return false;
    };
    if point.position == position {
        return false;
    }

    point.position = position;
    bump_revision(revision);
    true
}

/// Moves multiple stroke samples and captured fill vertices as one mutation batch.
pub fn move_samples(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    samples: &[(Uuid, usize, Vec2)],
    fill_points: &[(Uuid, usize, usize, Vec2)],
) -> bool {
    let mut changed = false;
    for &(stroke_id, sample_index, position) in samples {
        if !position.is_finite() {
            continue;
        }
        let Some(point) = drawing
            .strokes
            .iter_mut()
            .find(|stroke| stroke.id == stroke_id)
            .and_then(|stroke| stroke.points.get_mut(sample_index))
        else {
            continue;
        };
        if point.position != position {
            point.position = position;
            changed = true;
        }
    }
    for &(fill_id, boundary_index, point_index, position) in fill_points {
        if !position.is_finite() {
            continue;
        }
        let Some(point) = drawing
            .fills
            .iter_mut()
            .find(|fill| fill.id == fill_id)
            .and_then(|fill| fill.loops.get_mut(boundary_index))
            .and_then(|boundary| boundary.get_mut(point_index))
        else {
            continue;
        };
        if *point != position {
            *point = position;
            changed = true;
        }
    }
    if changed {
        bump_revision(revision);
    }
    changed
}

/// Removes one stored raw sample. Removing the final sample removes its stroke.
pub fn remove_sample(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    stroke_id: Uuid,
    sample_index: usize,
) -> bool {
    let Some(stroke_index) = drawing
        .strokes
        .iter()
        .position(|stroke| stroke.id == stroke_id)
    else {
        return false;
    };
    if sample_index >= drawing.strokes[stroke_index].points.len() {
        return false;
    }
    drawing.strokes[stroke_index].points.remove(sample_index);
    if drawing.strokes[stroke_index].points.is_empty() {
        drawing.strokes.remove(stroke_index);
    }
    bump_revision(revision);
    true
}

/// Removes multiple stroke samples and captured fill vertices as one mutation batch.
pub fn remove_samples(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    samples: &[(Uuid, usize)],
    fill_points: &[(Uuid, usize, usize)],
) -> bool {
    if samples.is_empty() && fill_points.is_empty() {
        return false;
    }
    let mut changed = false;
    for stroke in &mut drawing.strokes {
        let mut indices: Vec<_> = samples
            .iter()
            .filter(|(stroke_id, _)| *stroke_id == stroke.id)
            .map(|(_, index)| *index)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            if index < stroke.points.len() {
                stroke.points.remove(index);
                changed = true;
            }
        }
    }
    drawing.strokes.retain(|stroke| !stroke.points.is_empty());
    for fill in &mut drawing.fills {
        for (boundary_index, boundary) in fill.loops.iter_mut().enumerate() {
            let mut indices: Vec<_> = fill_points
                .iter()
                .filter(|(fill_id, selected_boundary, _)| {
                    *fill_id == fill.id && *selected_boundary == boundary_index
                })
                .map(|(_, _, point_index)| *point_index)
                .collect();
            indices.sort_unstable();
            indices.dedup();
            for index in indices.into_iter().rev() {
                if index < boundary.len() {
                    boundary.remove(index);
                    changed = true;
                }
            }
        }
        fill.loops.retain(|boundary| boundary.len() >= 3);
    }
    drawing.fills.retain(|fill| {
        !fill.loops.is_empty()
            || !fill_points
                .iter()
                .any(|(fill_id, _, _)| *fill_id == fill.id)
    });
    if !changed {
        return false;
    }
    bump_revision(revision);
    true
}

/// Removes complete fills and strokes in one mutation batch.
pub fn erase_objects(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    stroke_ids: &[Uuid],
    fill_ids: &[Uuid],
) -> bool {
    let before_fills = drawing.fills.len();
    drawing.fills.retain(|fill| !fill_ids.contains(&fill.id));
    let before_strokes = drawing.strokes.len();
    drawing
        .strokes
        .retain(|stroke| !stroke_ids.contains(&stroke.id));
    let changed = drawing.fills.len() != before_fills || drawing.strokes.len() != before_strokes;
    if changed {
        bump_revision(revision);
    }
    changed
}

/// Captures the prepared face containing `raw_seed`, or removes a captured fill
/// under that point. Captured loops are stored before Stroke Transform so later
/// transforms move the fill without rebuilding it from changed strokes.
pub fn toggle_fill(
    drawing: &mut PaintDrawing,
    revision: &mut u64,
    geometry: &PreparedGeometry,
    raw_seed: Vec2,
    stroke_transform: Mat3,
    color_index: usize,
) -> bool {
    assert_eq!(
        geometry.key.centerlines.revision, *revision,
        "prepared paint geometry is stale"
    );
    if !raw_seed.is_finite() {
        return false;
    }

    let prepared_seed = stroke_transform.transform_point2(raw_seed);
    let represented: Vec<_> = geometry
        .fills
        .iter()
        .filter(|fill| shrimply_paint_geometry::point_in_even_odd_loops(prepared_seed, &fill.loops))
        .map(|fill| fill.fill_id)
        .collect();

    if represented.is_empty() {
        let Some(face) = geometry.topology.face_at(prepared_seed) else {
            return false;
        };
        let inverse = stroke_transform.inverse();
        if !inverse.is_finite() {
            return false;
        }
        let loops = face
            .loops()
            .iter()
            .map(|boundary| {
                boundary
                    .iter()
                    .map(|point| inverse.transform_point2(*point))
                    .collect()
            })
            .collect();
        drawing
            .fills
            .push(PaintFill::new(raw_seed, loops, color_index));
    } else {
        drawing.fills.retain(|fill| !represented.contains(&fill.id));
    }
    bump_revision(revision);
    true
}

fn normalize(mut point: PaintPoint) -> Option<PaintPoint> {
    if !point.position.is_finite() {
        return None;
    }
    point.pressure = point
        .pressure
        .filter(|pressure| pressure.is_finite())
        .map(|pressure| pressure.clamp(0.0, 1.0));
    Some(point)
}

fn append_distinct(points: &mut Vec<PaintPoint>, samples: &[PaintPoint]) -> bool {
    let mut changed = false;
    for &sample in samples {
        if points.last().is_some_and(|previous| *previous == sample) {
            continue;
        }
        points.push(sample);
        changed = true;
    }
    changed
}

fn bump_revision(revision: &mut u64) {
    *revision = revision.checked_add(1).expect("paint revision overflow");
}
