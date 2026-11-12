//! Shared 2D geometry, fills, outlines and shadows. No editor state or raster surfaces.
use shrimply_math_color::Color;
use shrimply_math_geometry::ShapeKind;
pub use shrimply_math_geometry::ShapeRoundingStrategy;
use skia_safe::{
    BlurStyle, Canvas, MaskFilter, Paint, PaintStyle, Path, PathBuilder, PathDirection, Point, Rect,
};

#[derive(Clone, Copy)]
pub struct ShapeDraw {
    pub shape: ShapeKind,
    pub size: glam::Vec2,
    pub star_points: u32,
    pub star_inner_radius: f32,
    pub arrow_shaft_width: f32,
    pub arrow_head_length: f32,
    pub cross_arm_thickness: f32,
    pub ellipse_inner_radius: f32,
    pub ellipse_completion_degrees: f32,
    pub fill: Color<u8>,
    pub outline_color: Color<u8>,
    pub outline_width: f32,
    pub corner_radius: f32,
    pub rounding_strategy: ShapeRoundingStrategy,
    pub shadow_color: Color<u8>,
    pub shadow_offset: glam::Vec2,
    pub shadow_width: f32,
    pub shadow_blur: f32,
}

pub fn shape_path(shape: &ShapeDraw, rect: Rect) -> Path {
    match shape.shape {
        ShapeKind::Rect => rect_path(rect, shape.corner_radius, shape.rounding_strategy),
        ShapeKind::Ellipse => ellipse_path(
            shape.size,
            shape.ellipse_inner_radius,
            shape.ellipse_completion_degrees,
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Triangle => triangle_path(rect, shape.corner_radius, shape.rounding_strategy),
        ShapeKind::Star => polygon_path(
            shrimply_math_geometry::fit_vertices(
                shrimply_math_geometry::star_vertices(
                    shape.star_points,
                    shape.star_inner_radius,
                    -std::f32::consts::FRAC_PI_2,
                ),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Arrow => polygon_path(
            shrimply_math_geometry::arrow_vertices(
                shape.size,
                shape.arrow_shaft_width,
                shape.arrow_head_length,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Diamond => polygon_path(
            shrimply_math_geometry::fit_vertices(
                shrimply_math_geometry::regular_polygon_vertices(4, -std::f32::consts::FRAC_PI_2),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Pentagon => polygon_path(
            shrimply_math_geometry::fit_vertices(
                shrimply_math_geometry::regular_polygon_vertices(5, -std::f32::consts::FRAC_PI_2),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Hexagon => polygon_path(
            shrimply_math_geometry::fit_vertices(
                shrimply_math_geometry::regular_polygon_vertices(6, 0.0),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Heart => heart_path(rect),
        ShapeKind::Octagon => polygon_path(
            shrimply_math_geometry::fit_vertices(
                shrimply_math_geometry::regular_polygon_vertices(8, std::f32::consts::FRAC_PI_8),
                glam::Vec2::ZERO,
                shape.size,
            ),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
        ShapeKind::Cross => polygon_path(
            shrimply_math_geometry::cross_vertices(shape.size, shape.cross_arm_thickness),
            shape.corner_radius,
            shape.rounding_strategy,
        ),
    }
}

pub fn ellipse_path(
    size: glam::Vec2,
    inner_radius: f32,
    completion_degrees: f32,
    corner_radius: f32,
    rounding_strategy: ShapeRoundingStrategy,
) -> Path {
    let Some(segment) = shrimply_math_geometry::ellipse_segment(size, completion_degrees) else {
        return Path::new();
    };
    let center = Point::new(segment.center.x, segment.center.y);
    let radius = Point::new(segment.radius.x, segment.radius.y);
    let oval = Rect::from_xywh(
        center.x - radius.x,
        center.y - radius.y,
        radius.x * 2.0,
        radius.y * 2.0,
    );
    if segment.sweep_radians >= std::f32::consts::TAU - f32::EPSILON {
        let mut builder = PathBuilder::new();
        builder.add_oval(oval, PathDirection::CW, None);
        if inner_radius > f32::EPSILON {
            let inner = Rect::from_xywh(
                center.x - radius.x * inner_radius,
                center.y - radius.y * inner_radius,
                radius.x * inner_radius * 2.0,
                radius.y * inner_radius * 2.0,
            );
            builder.add_oval(inner, PathDirection::CCW, None);
        }
        return builder.detach();
    }
    let start_degrees = segment.start_radians.to_degrees();
    let sweep_degrees = segment.sweep_radians.to_degrees();
    let point = |angle: f32, scale: f32| {
        Point::new(
            center.x + angle.cos() * radius.x * scale,
            center.y + angle.sin() * radius.y * scale,
        )
    };
    let start = segment.start_radians;
    let end = start + segment.sweep_radians;
    let start_radius_length = glam::Vec2::new(
        start.cos() * segment.radius.x,
        start.sin() * segment.radius.y,
    )
    .length();
    let end_radius_length =
        glam::Vec2::new(end.cos() * segment.radius.x, end.sin() * segment.radius.y).length();
    let radial_length = start_radius_length.min(end_radius_length) * (1.0 - inner_radius);
    let trim = corner_radius.min(radial_length * 0.45);
    let outer_speed = (glam::Vec2::new(
        start.sin() * segment.radius.x,
        start.cos() * segment.radius.y,
    )
    .length()
        + glam::Vec2::new(end.sin() * segment.radius.x, end.cos() * segment.radius.y).length())
        * 0.5;
    let outer_trim_angle = (trim / outer_speed.max(f32::EPSILON)).min(segment.sweep_radians * 0.45);
    let start_radial_trim = trim / start_radius_length.max(f32::EPSILON);
    let end_radial_trim = trim / end_radius_length.max(f32::EPSILON);
    let mut builder = PathBuilder::new();
    if trim > f32::EPSILON {
        builder.move_to(point(start + outer_trim_angle, 1.0));
        builder.arc_to(
            oval,
            start_degrees + outer_trim_angle.to_degrees(),
            sweep_degrees - outer_trim_angle.to_degrees() * 2.0,
            false,
        );
        rounded_ellipse_corner(
            &mut builder,
            point(end - outer_trim_angle, 1.0),
            point(end, 1.0),
            point(end, 1.0 - end_radial_trim),
            rounding_strategy,
        );
    } else {
        builder.arc_to(oval, start_degrees, sweep_degrees, true);
    }
    if inner_radius > f32::EPSILON {
        let inner_radii = Point::new(radius.x * inner_radius, radius.y * inner_radius);
        let inner_oval = Rect::from_xywh(
            center.x - inner_radii.x,
            center.y - inner_radii.y,
            inner_radii.x * 2.0,
            inner_radii.y * 2.0,
        );
        if trim > f32::EPSILON {
            let inner_trim =
                trim.min(segment.radius.length() * inner_radius * segment.sweep_radians * 0.45);
            let inner_speed =
                (glam::Vec2::new(start.sin() * inner_radii.x, start.cos() * inner_radii.y)
                    .length()
                    + glam::Vec2::new(end.sin() * inner_radii.x, end.cos() * inner_radii.y)
                        .length())
                    * 0.5;
            let inner_trim_angle =
                (inner_trim / inner_speed.max(f32::EPSILON)).min(segment.sweep_radians * 0.45);
            builder.line_to(point(end, inner_radius + end_radial_trim));
            rounded_ellipse_corner(
                &mut builder,
                point(end, inner_radius + end_radial_trim),
                point(end, inner_radius),
                point(end - inner_trim_angle, inner_radius),
                rounding_strategy,
            );
            builder.arc_to(
                inner_oval,
                end.to_degrees() - inner_trim_angle.to_degrees(),
                -sweep_degrees + inner_trim_angle.to_degrees() * 2.0,
                false,
            );
            rounded_ellipse_corner(
                &mut builder,
                point(start + inner_trim_angle, inner_radius),
                point(start, inner_radius),
                point(start, inner_radius + start_radial_trim),
                rounding_strategy,
            );
            builder.line_to(point(start, 1.0 - start_radial_trim));
            rounded_ellipse_corner(
                &mut builder,
                point(start, 1.0 - start_radial_trim),
                point(start, 1.0),
                point(start + outer_trim_angle, 1.0),
                rounding_strategy,
            );
        } else {
            builder.arc_to(
                inner_oval,
                start_degrees + sweep_degrees,
                -sweep_degrees,
                false,
            );
        }
    } else if trim > f32::EPSILON {
        let center_trim = trim.min(start_radius_length.min(end_radius_length) * 0.45);
        builder.line_to(point(
            end,
            center_trim / end_radius_length.max(f32::EPSILON),
        ));
        rounded_ellipse_corner(
            &mut builder,
            point(end, center_trim / end_radius_length.max(f32::EPSILON)),
            center,
            point(start, center_trim / start_radius_length.max(f32::EPSILON)),
            rounding_strategy,
        );
        builder.line_to(point(start, 1.0 - start_radial_trim));
        rounded_ellipse_corner(
            &mut builder,
            point(start, 1.0 - start_radial_trim),
            point(start, 1.0),
            point(start + outer_trim_angle, 1.0),
            rounding_strategy,
        );
    } else {
        builder.line_to(center);
    }
    builder.close();
    builder.detach()
}

fn rounded_ellipse_corner(
    builder: &mut PathBuilder,
    entry: Point,
    corner: Point,
    exit: Point,
    strategy: ShapeRoundingStrategy,
) {
    match strategy {
        ShapeRoundingStrategy::Continuous => {
            builder.quad_to(corner, exit);
        }
        ShapeRoundingStrategy::Circular => {
            builder.conic_to(
                corner,
                exit,
                shrimply_math_geometry::corner_conic_weight(
                    glam::Vec2::new(entry.x - corner.x, entry.y - corner.y),
                    glam::Vec2::new(exit.x - corner.x, exit.y - corner.y),
                ),
            );
        }
        ShapeRoundingStrategy::Chamfer => {
            builder.line_to(exit);
        }
    }
}

pub fn draw_shape_path(canvas: &Canvas, shape: &ShapeDraw, path: &Path, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    if shape.shadow_offset.length_squared() > f32::EPSILON
        || shape.shadow_width > 0.0
        || shape.shadow_blur > 0.0
    {
        let mut shadow = fill_paint(shape.shadow_color, opacity);
        if shape.shadow_width > 0.0 {
            shadow.set_style(PaintStyle::StrokeAndFill);
            shadow.set_stroke_width(shape.shadow_width);
        }
        if shape.shadow_blur > 0.0 {
            shadow.set_mask_filter(MaskFilter::blur(BlurStyle::Normal, shape.shadow_blur, true));
        }
        canvas.save();
        canvas.translate((shape.shadow_offset.x, shape.shadow_offset.y));
        canvas.draw_path(path, &shadow);
        canvas.restore();
    }
    canvas.draw_path(path, &fill_paint(shape.fill, opacity));
    if shape.outline_width > 0.0 {
        let mut outline = fill_paint(shape.outline_color, opacity);
        outline.set_style(PaintStyle::Stroke);
        outline.set_stroke_width(shape.outline_width);
        canvas.draw_path(path, &outline);
    }
}

pub fn polygon_path(
    vertices: Vec<glam::Vec2>,
    corner_radius: f32,
    strategy: ShapeRoundingStrategy,
) -> Path {
    if corner_radius <= f32::EPSILON {
        return sharp_polygon_path(&vertices);
    }
    let circular = strategy == ShapeRoundingStrategy::Circular;
    let Some(corners) = shrimply_math_geometry::polygon_corners(&vertices, corner_radius, circular)
    else {
        return sharp_polygon_path(&vertices);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(corners[0].exit.x, corners[0].exit.y));
    for (index, corner) in corners
        .iter()
        .enumerate()
        .skip(1)
        .chain(corners.iter().enumerate().take(1))
    {
        builder.line_to(Point::new(corner.entry.x, corner.entry.y));
        let vertex = vertices[index];
        match strategy {
            ShapeRoundingStrategy::Continuous => {
                builder.quad_to(
                    Point::new(vertex.x, vertex.y),
                    Point::new(corner.exit.x, corner.exit.y),
                );
            }
            ShapeRoundingStrategy::Circular => {
                builder.conic_to(
                    Point::new(vertex.x, vertex.y),
                    Point::new(corner.exit.x, corner.exit.y),
                    corner.conic_weight,
                );
            }
            ShapeRoundingStrategy::Chamfer => {
                builder.line_to(Point::new(corner.exit.x, corner.exit.y));
            }
        }
    }
    builder.close();
    builder.detach()
}

fn sharp_polygon_path(vertices: &[glam::Vec2]) -> Path {
    let mut builder = PathBuilder::new();
    let Some(first) = vertices.first() else {
        return builder.detach();
    };
    builder.move_to(Point::new(first.x, first.y));
    for vertex in &vertices[1..] {
        builder.line_to(Point::new(vertex.x, vertex.y));
    }
    builder.close();
    builder.detach()
}

pub fn heart_path(rect: Rect) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.center_x(), rect.bottom()));
    builder.cubic_to(
        Point::new(
            rect.left() + rect.width() * 0.45,
            rect.top() + rect.height() * 0.88,
        ),
        Point::new(rect.left(), rect.top() + rect.height() * 0.62),
        Point::new(rect.left(), rect.top() + rect.height() * 0.32),
    );
    builder.cubic_to(
        Point::new(rect.left(), rect.top() + rect.height() * 0.08),
        Point::new(rect.left() + rect.width() * 0.28, rect.top()),
        Point::new(rect.center_x(), rect.top() + rect.height() * 0.22),
    );
    builder.cubic_to(
        Point::new(rect.left() + rect.width() * 0.72, rect.top()),
        Point::new(rect.right(), rect.top() + rect.height() * 0.08),
        Point::new(rect.right(), rect.top() + rect.height() * 0.32),
    );
    builder.cubic_to(
        Point::new(rect.right(), rect.top() + rect.height() * 0.62),
        Point::new(
            rect.left() + rect.width() * 0.55,
            rect.top() + rect.height() * 0.88,
        ),
        Point::new(rect.center_x(), rect.bottom()),
    );
    builder.close();
    builder.detach()
}

pub fn rect_path(rect: Rect, corner_radius: f32, strategy: ShapeRoundingStrategy) -> Path {
    match strategy {
        ShapeRoundingStrategy::Chamfer => chamfer_rect_path(rect, corner_radius),
        ShapeRoundingStrategy::Circular => Path::rrect(
            skia_safe::RRect::new_rect_xy(rect, corner_radius.max(0.0), corner_radius.max(0.0)),
            None,
        ),
        ShapeRoundingStrategy::Continuous => continuous_rect_path(rect, corner_radius),
    }
}

fn continuous_rect_path(rect: Rect, corner_radius: f32) -> Path {
    let radius = rect_corner_radius(rect, corner_radius);
    if radius <= f32::EPSILON {
        return sharp_rect_path(rect);
    }
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left() + radius, rect.top()));
    builder.line_to(Point::new(rect.right() - radius, rect.top()));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.right() - radius, rect.top() + radius),
        radius,
        -std::f32::consts::FRAC_PI_2,
        0.0,
    );
    builder.line_to(Point::new(rect.right(), rect.bottom() - radius));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.right() - radius, rect.bottom() - radius),
        radius,
        0.0,
        std::f32::consts::FRAC_PI_2,
    );
    builder.line_to(Point::new(rect.left() + radius, rect.bottom()));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.left() + radius, rect.bottom() - radius),
        radius,
        std::f32::consts::FRAC_PI_2,
        std::f32::consts::PI,
    );
    builder.line_to(Point::new(rect.left(), rect.top() + radius));
    add_continuous_rect_corner(
        &mut builder,
        Point::new(rect.left() + radius, rect.top() + radius),
        radius,
        std::f32::consts::PI,
        std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
    );
    builder.close();
    builder.detach()
}

fn add_continuous_rect_corner(
    builder: &mut PathBuilder,
    center: Point,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
) {
    const SAMPLES: u32 = 12;
    for step in 1..=SAMPLES {
        let amount = step as f32 / SAMPLES as f32;
        let angle = start_angle + (end_angle - start_angle) * amount;
        let cos = angle.cos();
        let sin = angle.sin();
        builder.line_to(Point::new(
            center.x + radius * cos.signum() * cos.abs().sqrt(),
            center.y + radius * sin.signum() * sin.abs().sqrt(),
        ));
    }
}

fn chamfer_rect_path(rect: Rect, corner_radius: f32) -> Path {
    let radius = rect_corner_radius(rect, corner_radius);
    if radius <= f32::EPSILON {
        return sharp_rect_path(rect);
    }
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left() + radius, rect.top()));
    builder.line_to(Point::new(rect.right() - radius, rect.top()));
    builder.line_to(Point::new(rect.right(), rect.top() + radius));
    builder.line_to(Point::new(rect.right(), rect.bottom() - radius));
    builder.line_to(Point::new(rect.right() - radius, rect.bottom()));
    builder.line_to(Point::new(rect.left() + radius, rect.bottom()));
    builder.line_to(Point::new(rect.left(), rect.bottom() - radius));
    builder.line_to(Point::new(rect.left(), rect.top() + radius));
    builder.close();
    builder.detach()
}

fn sharp_rect_path(rect: Rect) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(Point::new(rect.left(), rect.top()));
    builder.line_to(Point::new(rect.right(), rect.top()));
    builder.line_to(Point::new(rect.right(), rect.bottom()));
    builder.line_to(Point::new(rect.left(), rect.bottom()));
    builder.close();
    builder.detach()
}

fn rect_corner_radius(rect: Rect, corner_radius: f32) -> f32 {
    corner_radius
        .max(0.0)
        .min(rect.width() * 0.5)
        .min(rect.height() * 0.5)
}

pub fn triangle_path(rect: Rect, corner_radius: f32, strategy: ShapeRoundingStrategy) -> Path {
    let top = Point::new(rect.center_x(), rect.top());
    let right = Point::new(rect.right(), rect.bottom());
    let left = Point::new(rect.left(), rect.bottom());
    if corner_radius <= 0.0 {
        return sharp_triangle_path(top, right, left);
    }
    match strategy {
        ShapeRoundingStrategy::Continuous => {
            continuous_triangle_path(top, right, left, corner_radius)
        }
        ShapeRoundingStrategy::Circular => circular_triangle_path(top, right, left, corner_radius),
        ShapeRoundingStrategy::Chamfer => chamfer_triangle_path(top, right, left, corner_radius),
    }
}

fn circular_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = circular_triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = circular_triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = circular_triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.conic_to(right, right_corner.exit, right_corner.weight);
    builder.line_to(left_corner.entry);
    builder.conic_to(left, left_corner.exit, left_corner.weight);
    builder.line_to(top_corner.entry);
    builder.conic_to(top, top_corner.exit, top_corner.weight);
    builder.close();
    builder.detach()
}

fn continuous_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.quad_to(right, right_corner.exit);
    builder.line_to(left_corner.entry);
    builder.quad_to(left, left_corner.exit);
    builder.line_to(top_corner.entry);
    builder.quad_to(top, top_corner.exit);
    builder.close();
    builder.detach()
}

fn chamfer_triangle_path(top: Point, right: Point, left: Point, corner_radius: f32) -> Path {
    let Some(top_corner) = triangle_corner(left, top, right, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(right_corner) = triangle_corner(top, right, left, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let Some(left_corner) = triangle_corner(right, left, top, corner_radius) else {
        return sharp_triangle_path(top, right, left);
    };
    let mut builder = PathBuilder::new();
    builder.move_to(top_corner.exit);
    builder.line_to(right_corner.entry);
    builder.line_to(right_corner.exit);
    builder.line_to(left_corner.entry);
    builder.line_to(left_corner.exit);
    builder.line_to(top_corner.entry);
    builder.line_to(top_corner.exit);
    builder.close();
    builder.detach()
}

fn sharp_triangle_path(top: Point, right: Point, left: Point) -> Path {
    let mut builder = PathBuilder::new();
    builder.move_to(top);
    builder.line_to(right);
    builder.line_to(left);
    builder.close();
    builder.detach()
}

#[derive(Clone, Copy)]
struct TriangleCorner {
    entry: Point,
    exit: Point,
    weight: f32,
}

fn circular_triangle_corner(
    previous: Point,
    corner: Point,
    next: Point,
    radius: f32,
) -> Option<TriangleCorner> {
    let previous_x = previous.x - corner.x;
    let previous_y = previous.y - corner.y;
    let next_x = next.x - corner.x;
    let next_y = next.y - corner.y;
    let previous_length = previous_x.hypot(previous_y);
    let next_length = next_x.hypot(next_y);
    if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
        return None;
    }
    let dot = ((previous_x * next_x + previous_y * next_y) / (previous_length * next_length))
        .clamp(-1.0, 1.0);
    let half_angle = dot.acos() * 0.5;
    let tangent = half_angle.tan();
    if tangent <= f32::EPSILON {
        return None;
    }
    let distance = (radius / tangent)
        .min(previous_length * 0.45)
        .min(next_length * 0.45);
    let corner = triangle_corner_at_distance(
        previous,
        corner,
        next,
        previous_length,
        next_length,
        distance,
    )?;
    Some(TriangleCorner {
        weight: half_angle.cos().max(f32::EPSILON),
        ..corner
    })
}

fn triangle_corner(
    previous: Point,
    corner: Point,
    next: Point,
    distance: f32,
) -> Option<TriangleCorner> {
    let previous_length = (previous.x - corner.x).hypot(previous.y - corner.y);
    let next_length = (next.x - corner.x).hypot(next.y - corner.y);
    triangle_corner_at_distance(
        previous,
        corner,
        next,
        previous_length,
        next_length,
        distance,
    )
}

fn triangle_corner_at_distance(
    previous: Point,
    corner: Point,
    next: Point,
    previous_length: f32,
    next_length: f32,
    distance: f32,
) -> Option<TriangleCorner> {
    if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
        return None;
    }
    let distance = distance.min(previous_length * 0.45).min(next_length * 0.45);
    if distance <= f32::EPSILON {
        return None;
    }
    Some(TriangleCorner {
        entry: point_on_edge(corner, previous, previous_length, distance),
        exit: point_on_edge(corner, next, next_length, distance),
        weight: 1.0,
    })
}

fn point_on_edge(from: Point, to: Point, length: f32, distance: f32) -> Point {
    let amount = distance / length;
    Point::new(
        from.x + (to.x - from.x) * amount,
        from.y + (to.y - from.y) * amount,
    )
}

pub fn fill_paint(color: Color<u8>, opacity: f32) -> Paint {
    paint(
        skia_safe::Color4f::from(skia_safe::Color::from(color)),
        opacity,
    )
}

/// Common RGBA paint setup for editor and DSL vector sources.
pub fn paint(mut color: skia_safe::Color4f, opacity: f32) -> Paint {
    color.a *= opacity;
    let mut paint = Paint::default();
    paint.set_anti_alias(true).set_color4f(color, None);
    paint
}
