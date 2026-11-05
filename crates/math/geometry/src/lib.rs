pub use glam::{DVec2, IVec2, Mat3, USizeVec2, UVec2, Vec2, Vec3, vec2, vec3};
use serde::{Deserialize, Serialize};

mod camera_reconstruction;
mod ellipse;
mod morph;
mod projective;
#[cfg(feature = "skia")]
mod skia;
#[cfg(feature = "skia")]
pub use skia::*;
pub mod snap;
mod stabilization;
mod transform;
pub use camera_reconstruction::*;
pub use ellipse::{EllipseSegment, ellipse_segment};
pub use morph::*;
pub use projective::*;
pub use stabilization::*;
pub use transform::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self::from_min_size(Vec2::new(x, y), Vec2::new(width, height))
    }

    pub fn from_min_size(min: Vec2, size: Vec2) -> Self {
        Self {
            min,
            max: min + size.max(Vec2::ZERO),
        }
    }

    pub fn from_center_size(center: Vec2, size: Vec2) -> Self {
        Self::from_min_size(center - size.max(Vec2::ZERO) * 0.5, size)
    }

    pub const fn from_min_max(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }

    pub fn size(self) -> Vec2 {
        (self.max - self.min).max(Vec2::ZERO)
    }

    pub fn center(self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    pub fn contains(self, point: Vec2) -> bool {
        point.cmpge(self.min).all() && point.cmple(self.max).all()
    }

    pub fn wrap_point(self, point: Vec2) -> Vec2 {
        let size = self.size();
        Vec2::new(
            if size.x > 0.0 {
                self.min.x + (point.x - self.min.x).rem_euclid(size.x)
            } else {
                self.min.x
            },
            if size.y > 0.0 {
                self.min.y + (point.y - self.min.y).rem_euclid(size.y)
            } else {
                self.min.y
            },
        )
    }

    pub fn expand(self, amount: f32) -> Self {
        self.outset(Vec2::splat(amount))
    }

    pub fn outset(self, amount: Vec2) -> Self {
        Self {
            min: self.min - amount,
            max: self.max + amount,
        }
    }

    pub fn translated(self, offset: Vec2) -> Self {
        Self {
            min: self.min + offset,
            max: self.max + offset,
        }
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn width(self) -> f32 {
        self.size().x
    }

    pub fn height(self) -> f32 {
        self.size().y
    }

    pub fn left(self) -> f32 {
        self.min.x
    }

    pub fn right(self) -> f32 {
        self.max.x
    }

    pub fn top(self) -> f32 {
        self.min.y
    }

    pub fn bottom(self) -> f32 {
        self.max.y
    }
}

pub fn distance_to_segment(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    if segment.length_squared() <= f32::EPSILON {
        return point.distance(start);
    }
    let amount = ((point - start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0);
    point.distance(start + segment * amount)
}

pub fn distance_to_dsegment(point: DVec2, start: DVec2, end: DVec2) -> f64 {
    let segment = end - start;
    if segment.length_squared() <= f64::EPSILON {
        return point.distance(start);
    }
    let amount = ((point - start).dot(segment) / segment.length_squared()).clamp(0.0, 1.0);
    point.distance(start + segment * amount)
}

pub fn regular_polygon_vertices(sides: u32, rotation: f32) -> Vec<Vec2> {
    (0..sides)
        .map(|index| {
            let angle = rotation + std::f32::consts::TAU * index as f32 / sides as f32;
            Vec2::new(angle.cos(), angle.sin())
        })
        .collect()
}

pub fn star_vertices(points: u32, inner_radius: f32, rotation: f32) -> Vec<Vec2> {
    (0..points * 2)
        .map(|index| {
            let angle = rotation + std::f32::consts::PI * index as f32 / points as f32;
            let radius = if index % 2 == 0 { 1.0 } else { inner_radius };
            Vec2::new(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

pub fn fit_vertices(mut vertices: Vec<Vec2>, origin: Vec2, size: Vec2) -> Vec<Vec2> {
    let (mut minimum, mut maximum) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for vertex in &vertices {
        minimum = minimum.min(*vertex);
        maximum = maximum.max(*vertex);
    }
    let extent = (maximum - minimum).max(Vec2::splat(f32::EPSILON));
    for vertex in &mut vertices {
        *vertex = origin + (*vertex - minimum) / extent * size;
    }
    vertices
}

pub fn arrow_vertices(size: Vec2, shaft_width: f32, head_length: f32) -> Vec<Vec2> {
    let center_y = size.y * 0.5;
    let shaft_half_height = size.y * shaft_width * 0.5;
    let head_left = size.x * (1.0 - head_length);
    vec![
        Vec2::new(0.0, center_y - shaft_half_height),
        Vec2::new(head_left, center_y - shaft_half_height),
        Vec2::new(head_left, 0.0),
        Vec2::new(size.x, center_y),
        Vec2::new(head_left, size.y),
        Vec2::new(head_left, center_y + shaft_half_height),
        Vec2::new(0.0, center_y + shaft_half_height),
    ]
}

pub fn cross_vertices(size: Vec2, arm_thickness: f32) -> Vec<Vec2> {
    let horizontal_inset = size.x * (1.0 - arm_thickness) * 0.5;
    let vertical_inset = size.y * (1.0 - arm_thickness) * 0.5;
    vec![
        Vec2::new(horizontal_inset, 0.0),
        Vec2::new(size.x - horizontal_inset, 0.0),
        Vec2::new(size.x - horizontal_inset, vertical_inset),
        Vec2::new(size.x, vertical_inset),
        Vec2::new(size.x, size.y - vertical_inset),
        Vec2::new(size.x - horizontal_inset, size.y - vertical_inset),
        Vec2::new(size.x - horizontal_inset, size.y),
        Vec2::new(horizontal_inset, size.y),
        Vec2::new(horizontal_inset, size.y - vertical_inset),
        Vec2::new(0.0, size.y - vertical_inset),
        Vec2::new(0.0, vertical_inset),
        Vec2::new(horizontal_inset, vertical_inset),
    ]
}

/// Largest scale applied by the affine part of a 2D homogeneous transform.
pub fn max_affine_scale(matrix: Mat3) -> f32 {
    let values = matrix.to_cols_array();
    let (a, b, c, d) = (values[0], values[1], values[3], values[4]);
    let trace = a * a + b * b + c * c + d * d;
    let determinant = (a * d - b * c).powi(2);
    ((trace + (trace * trace - 4.0 * determinant).max(0.0).sqrt()) * 0.5).sqrt()
}

pub use morph::{FLOW_GRID_SPACING, optical_flow_grid_size};
