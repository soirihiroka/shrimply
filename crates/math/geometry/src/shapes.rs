use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    #[default]
    Rect,
    Ellipse,
    Triangle,
    Star,
    Arrow,
    Diamond,
    Pentagon,
    Hexagon,
    Heart,
    Octagon,
    Cross,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShapeRoundingStrategy {
    #[default]
    Continuous,
    Circular,
    Chamfer,
}

#[derive(Clone, Copy)]
pub struct PolygonCorner {
    pub entry: glam::Vec2,
    pub exit: glam::Vec2,
    pub conic_weight: f32,
}

pub fn polygon_corners(
    vertices: &[glam::Vec2],
    corner_radius: f32,
    circular: bool,
) -> Option<Vec<PolygonCorner>> {
    if vertices.len() < 3 {
        return None;
    }
    vertices
        .iter()
        .enumerate()
        .map(|(index, &corner)| {
            let previous = vertices[(index + vertices.len() - 1) % vertices.len()];
            let next = vertices[(index + 1) % vertices.len()];
            let previous_edge = previous - corner;
            let next_edge = next - corner;
            let previous_length = previous_edge.length();
            let next_length = next_edge.length();
            if previous_length <= f32::EPSILON || next_length <= f32::EPSILON {
                return None;
            }
            let half_angle = corner_half_angle(previous_edge, next_edge);
            let distance = if circular {
                let tangent = half_angle.tan();
                if tangent <= f32::EPSILON {
                    return None;
                }
                corner_radius / tangent
            } else {
                corner_radius
            }
            .min(previous_length * 0.45)
            .min(next_length * 0.45);
            if distance <= f32::EPSILON {
                return None;
            }
            Some(PolygonCorner {
                entry: corner + previous_edge * (distance / previous_length),
                exit: corner + next_edge * (distance / next_length),
                conic_weight: if circular {
                    half_angle.cos().max(f32::EPSILON)
                } else {
                    1.0
                },
            })
        })
        .collect()
}

pub fn corner_conic_weight(previous_edge: glam::Vec2, next_edge: glam::Vec2) -> f32 {
    corner_half_angle(previous_edge, next_edge)
        .cos()
        .max(f32::EPSILON)
}

fn corner_half_angle(previous_edge: glam::Vec2, next_edge: glam::Vec2) -> f32 {
    (previous_edge.dot(next_edge) / (previous_edge.length() * next_edge.length()))
        .clamp(-1.0, 1.0)
        .acos()
        * 0.5
}
