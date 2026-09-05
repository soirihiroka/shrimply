use std::rc::Rc;

use skia_safe::{
    BlendMode, Canvas, Paint, Path, PathBuilder, PathFillType, PathVerb, Point,
    canvas::SaveLayerRec,
};

use crate::generated::GeneratedVisual;
use crate::generated::VectorOperation;

#[derive(Clone)]
pub struct MorphPaintLayer {
    pub paint: Paint,
    pub offset: glam::Vec2,
}

#[derive(Clone)]
pub struct MorphObject {
    pub path: shrimply_math_geometry::MorphPath,
    pub appearance: Vec<MorphPaintLayer>,
}

#[derive(Clone)]
pub struct MorphScene {
    pub objects: Vec<MorphObject>,
    pub evaluation: shrimply_evaluation::VisualEvaluation,
    pub canvas_size: shrimply_project::project::CanvasSize,
}

pub struct PreparedVectorMorph {
    source: MorphScene,
    target: MorphScene,
    matching: shrimply_math_geometry::MorphMatching,
    source_center: glam::Vec2,
    target_center: glam::Vec2,
}

pub struct MorphFrame {
    morph: Rc<PreparedVectorMorph>,
    progress: f32,
}

impl PreparedVectorMorph {
    pub fn new(source: MorphScene, target: MorphScene) -> Self {
        let source_center = scene_center(&source);
        let target_center = scene_center(&target);
        let matching = shrimply_math_geometry::match_morph_paths(
            &source
                .objects
                .iter()
                .map(|object| object.path.clone())
                .collect::<Vec<_>>(),
            &target
                .objects
                .iter()
                .map(|object| object.path.clone())
                .collect::<Vec<_>>(),
        );
        Self {
            source,
            target,
            matching,
            source_center,
            target_center,
        }
    }

    pub fn frame(self: &Rc<Self>, progress: f32) -> MorphFrame {
        MorphFrame {
            morph: Rc::clone(self),
            progress: progress.clamp(0.0, 1.0),
        }
    }

    pub fn source(&self) -> &MorphScene {
        &self.source
    }

    pub fn target(&self) -> &MorphScene {
        &self.target
    }
}

impl GeneratedVisual for MorphFrame {
    fn draw(
        &self,
        canvas: &Canvas,
        _evaluation: &shrimply_evaluation::TransformEvaluation,
        _expressions: &mut shrimply_evaluation::TransformExpressionCache,
        _path_effect: Option<&skia_safe::PathEffect>,
    ) {
        canvas.save_layer_alpha_f(None, 1.0 - self.progress);
        self.draw_side(canvas, true);
        canvas.restore();
        let mut target_blend = Paint::default();
        target_blend.set_alpha_f(self.progress);
        target_blend.set_blend_mode(BlendMode::Plus);
        canvas.save_layer(&SaveLayerRec::default().paint(&target_blend));
        self.draw_side(canvas, false);
        canvas.restore();
    }
}

impl MorphFrame {
    fn draw_side(&self, canvas: &Canvas, source: bool) {
        for pair in &self.morph.matching.pairs {
            let path = shrimply_math_geometry::interpolate_morph_path(
                &pair.source_path,
                &pair.target_path,
                self.progress,
            );
            let object = if source {
                &self.morph.source.objects[pair.source]
            } else {
                &self.morph.target.objects[pair.target]
            };
            draw_object(canvas, &path, &object.appearance);
        }
        if source {
            for &index in &self.morph.matching.unmatched_source {
                let object = &self.morph.source.objects[index];
                let path = shrimply_math_geometry::collapse_morph_path(
                    &object.path,
                    self.morph.target_center,
                    self.progress,
                );
                draw_object(canvas, &path, &object.appearance);
            }
        } else {
            for &index in &self.morph.matching.unmatched_target {
                let object = &self.morph.target.objects[index];
                let path = shrimply_math_geometry::collapse_morph_path(
                    &object.path,
                    self.morph.source_center,
                    1.0 - self.progress,
                );
                draw_object(canvas, &path, &object.appearance);
            }
        }
    }
}

pub fn apply_vector_operations(
    mut scene: MorphScene,
    operations: &[VectorOperation],
) -> Option<MorphScene> {
    for operation in operations {
        match operation {
            VectorOperation::Transform(transform) => {
                let matrix = shrimply_math_geometry::to_skia_matrix(transform.matrix);
                let scale = transform
                    .matrix
                    .x_axis
                    .truncate()
                    .length()
                    .max(transform.matrix.y_axis.truncate().length());
                for object in &mut scene.objects {
                    object.path = transform_morph_path(&object.path, transform.matrix);
                    for layer in &mut object.appearance {
                        layer.offset = transform.matrix.transform_vector2(layer.offset);
                        layer
                            .paint
                            .set_stroke_width(layer.paint.stroke_width() * scale);
                        if let Some(shader) = layer.paint.shader() {
                            layer.paint.set_shader(shader.with_local_matrix(&matrix));
                        }
                    }
                }
            }
            VectorOperation::Opacity(opacity) => {
                for object in &mut scene.objects {
                    for layer in &mut object.appearance {
                        layer.paint.set_alpha_f(layer.paint.alpha_f() * opacity);
                    }
                }
            }
            VectorOperation::Hsv {
                hue_turns,
                saturation,
                value,
            } => {
                for object in &mut scene.objects {
                    for layer in &mut object.appearance {
                        let hsv = shrimply_preview_skia::hsv_color_filter(
                            *hue_turns,
                            *saturation,
                            *value,
                        );
                        let filter = if let Some(previous) = layer.paint.color_filter() {
                            hsv.composed(previous)
                                .expect("compose compatible vector color filters")
                        } else {
                            hsv
                        };
                        layer.paint.set_color_filter(filter);
                    }
                }
            }
            VectorOperation::Repeat {
                copies_x,
                copies_y,
                step,
                row_offset,
            } => {
                let original = scene.objects;
                let mut repeated =
                    Vec::with_capacity(original.len() * *copies_x as usize * *copies_y as usize);
                for row in 0..*copies_y {
                    for column in 0..*copies_x {
                        let offset = *step * glam::Vec2::new(column as f32, row as f32)
                            + *row_offset * row as f32;
                        repeated.extend(original.iter().cloned().map(|mut object| {
                            object.path = transform_morph_path(
                                &object.path,
                                glam::Mat3::from_translation(offset),
                            );
                            object
                        }));
                    }
                }
                scene.objects = repeated;
            }
            VectorOperation::ShakyPath { .. }
            | VectorOperation::MotionBlur(_)
            | VectorOperation::TextMask(_) => return None,
        }
    }
    Some(scene)
}

pub fn skia_path_to_morph(path: &Path) -> shrimply_math_geometry::MorphPath {
    let mut contours = Vec::new();
    let mut curves = Vec::new();
    let mut current = glam::Vec2::ZERO;
    let mut start = glam::Vec2::ZERO;
    let mut closed = false;
    let flush = |contours: &mut Vec<shrimply_math_geometry::MorphContour>,
                 curves: &mut Vec<[glam::Vec2; 4]>,
                 closed: bool| {
        if !curves.is_empty() {
            contours.push(shrimply_math_geometry::MorphContour {
                curves: std::mem::take(curves),
                closed,
            });
        }
    };
    for record in path.iter() {
        let points = record.points();
        match record.verb() {
            PathVerb::Move => {
                flush(&mut contours, &mut curves, closed);
                current = point(points[0]);
                start = current;
                closed = false;
            }
            PathVerb::Line => {
                let end = point(points[1]);
                curves.push([
                    current,
                    current.lerp(end, 1.0 / 3.0),
                    current.lerp(end, 2.0 / 3.0),
                    end,
                ]);
                current = end;
            }
            PathVerb::Quad | PathVerb::Conic => {
                let control = point(points[1]);
                let end = point(points[2]);
                curves.push([
                    current,
                    current + (control - current) * (2.0 / 3.0),
                    end + (control - end) * (2.0 / 3.0),
                    end,
                ]);
                current = end;
            }
            PathVerb::Cubic => {
                let first = point(points[1]);
                let second = point(points[2]);
                let end = point(points[3]);
                curves.push([current, first, second, end]);
                current = end;
            }
            PathVerb::Close => {
                if current.distance_squared(start) > f32::EPSILON {
                    curves.push([
                        current,
                        current.lerp(start, 1.0 / 3.0),
                        current.lerp(start, 2.0 / 3.0),
                        start,
                    ]);
                }
                current = start;
                closed = true;
            }
        }
    }
    flush(&mut contours, &mut curves, closed);
    let fill_type = match path.fill_type() {
        PathFillType::Winding => shrimply_math_geometry::MorphFillType::Winding,
        PathFillType::EvenOdd => shrimply_math_geometry::MorphFillType::EvenOdd,
        PathFillType::InverseWinding => shrimply_math_geometry::MorphFillType::InverseWinding,
        PathFillType::InverseEvenOdd => shrimply_math_geometry::MorphFillType::InverseEvenOdd,
    };
    shrimply_math_geometry::MorphPath {
        contours,
        fill_type,
    }
}

pub fn morph_to_skia_path(path: &shrimply_math_geometry::MorphPath) -> Path {
    let mut builder = PathBuilder::new();
    builder.set_fill_type(match path.fill_type {
        shrimply_math_geometry::MorphFillType::Winding => PathFillType::Winding,
        shrimply_math_geometry::MorphFillType::EvenOdd => PathFillType::EvenOdd,
        shrimply_math_geometry::MorphFillType::InverseWinding => PathFillType::InverseWinding,
        shrimply_math_geometry::MorphFillType::InverseEvenOdd => PathFillType::InverseEvenOdd,
    });
    for contour in &path.contours {
        let Some(first) = contour.curves.first() else {
            continue;
        };
        builder.move_to(Point::new(first[0].x, first[0].y));
        for curve in &contour.curves {
            builder.cubic_to(
                Point::new(curve[1].x, curve[1].y),
                Point::new(curve[2].x, curve[2].y),
                Point::new(curve[3].x, curve[3].y),
            );
        }
        if contour.closed {
            builder.close();
        }
    }
    builder.detach()
}

fn draw_object(
    canvas: &Canvas,
    path: &shrimply_math_geometry::MorphPath,
    appearance: &[MorphPaintLayer],
) {
    let path = morph_to_skia_path(path);
    for layer in appearance {
        canvas.save();
        canvas.translate((layer.offset.x, layer.offset.y));
        canvas.draw_path(&path, &layer.paint);
        canvas.restore();
    }
}

fn scene_center(scene: &MorphScene) -> glam::Vec2 {
    let mut points = scene.objects.iter().flat_map(|object| {
        object
            .path
            .contours
            .iter()
            .flat_map(|contour| contour.curves.iter().flatten().copied())
    });
    let Some(first) = points.next() else {
        return glam::Vec2::new(
            scene.canvas_size.width as f32 * 0.5,
            scene.canvas_size.height as f32 * 0.5,
        );
    };
    let (minimum, maximum) = points.fold((first, first), |(minimum, maximum), point| {
        (minimum.min(point), maximum.max(point))
    });
    (minimum + maximum) * 0.5
}

fn transform_morph_path(
    path: &shrimply_math_geometry::MorphPath,
    matrix: glam::Mat3,
) -> shrimply_math_geometry::MorphPath {
    shrimply_math_geometry::MorphPath {
        contours: path
            .contours
            .iter()
            .map(|contour| shrimply_math_geometry::MorphContour {
                curves: contour
                    .curves
                    .iter()
                    .map(|curve| curve.map(|point| matrix.transform_point2(point)))
                    .collect(),
                closed: contour.closed,
            })
            .collect(),
        fill_type: path.fill_type,
    }
}

fn point(point: Point) -> glam::Vec2 {
    glam::Vec2::new(point.x, point.y)
}
