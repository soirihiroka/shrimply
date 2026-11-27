use glam::{Vec2, Vec3};
use lyon_tessellation::{
    FillOptions, FillRule, FillTessellator, VertexBuffers,
    geometry_builder::{BuffersBuilder, Positions},
    math::{Point, point},
    path::Path,
};
use shrimply_scene_3d::{MeshMaterial, ObjMesh, TextureAtlas};

use crate::Shape3dError;

const MAX_BEVEL_MITER: f32 = 2.0;
const SMOOTH_NORMAL_DOT_THRESHOLD: f32 = 0.8;
const BASE_RADIAL_SEGMENTS: u32 = 8;
const RADIAL_SEGMENTS_PER_SMOOTHNESS: u32 = 6;
const BASE_VERTICAL_SEGMENTS: u32 = 4;
const VERTICAL_SEGMENTS_PER_SMOOTHNESS: u32 = 3;

pub(crate) fn extrude(
    contours: Vec<Vec<Vec2>>,
    depth: f32,
    requested_roundness: f32,
    smoothness: u32,
) -> Result<ObjMesh, Shape3dError> {
    if contours.is_empty() {
        return Ok(empty_mesh());
    }
    let mut path = Path::builder();
    for contour in &contours {
        let Some(first) = contour.first() else {
            continue;
        };
        path.begin(point(first.x, first.y));
        for vertex in &contour[1..] {
            path.line_to(point(vertex.x, vertex.y));
        }
        path.end(true);
    }
    let path = path.build();
    let mut fill = VertexBuffers::<Point, u32>::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::default().with_fill_rule(FillRule::NonZero),
            &mut BuffersBuilder::new(&mut fill, Positions),
        )
        .map_err(|error| Shape3dError(format!("tessellate 3D shape: {error:?}")))?;
    let half_depth = depth * 0.5;
    let requested_roundness = requested_roundness.clamp(0.0, half_depth);
    let roundness = safe_roundness(&contours, requested_roundness);
    let mut mesh = MeshBuilder::default();
    for triangle in fill.indices.as_chunks::<3>().0 {
        let points = [
            fill.vertices[triangle[0] as usize],
            fill.vertices[triangle[1] as usize],
            fill.vertices[triangle[2] as usize],
        ];
        mesh.cap(points, half_depth, true);
        mesh.cap(points, -half_depth, false);
    }
    for contour in contours {
        mesh.extrude_contour(&contour, half_depth, roundness, smoothness);
    }
    mesh.finish()
}

pub(crate) fn sphere(size: Vec3, smoothness: u32) -> Result<ObjMesh, Shape3dError> {
    let slices = BASE_RADIAL_SEGMENTS + smoothness * RADIAL_SEGMENTS_PER_SMOOTHNESS;
    let stacks = BASE_VERTICAL_SEGMENTS + smoothness * VERTICAL_SEGMENTS_PER_SMOOTHNESS;
    let radii = size * 0.5;
    parametric(slices, stacks, |u, v| {
        let longitude = std::f32::consts::TAU * u;
        let latitude = std::f32::consts::PI * (v - 0.5);
        let unit = Vec3::new(
            latitude.cos() * longitude.cos(),
            latitude.cos() * longitude.sin(),
            latitude.sin(),
        );
        let position = unit * radii;
        let normal = (unit / radii).normalize_or_zero();
        (position, normal)
    })
}

pub(crate) fn torus(
    size: Vec3,
    inner_percent: f32,
    smoothness: u32,
) -> Result<ObjMesh, Shape3dError> {
    let segments = BASE_RADIAL_SEGMENTS + smoothness * RADIAL_SEGMENTS_PER_SMOOTHNESS;
    let rings = BASE_RADIAL_SEGMENTS + smoothness * RADIAL_SEGMENTS_PER_SMOOTHNESS;
    let outer = size.truncate() * 0.5;
    let inner = outer * inner_percent;
    let major = (outer + inner) * 0.5;
    let tube = Vec3::new(
        (outer.x - inner.x) * 0.5,
        (outer.y - inner.y) * 0.5,
        size.z * 0.5,
    );
    parametric(segments, rings, |u, v| {
        let around = std::f32::consts::TAU * u;
        let cross = std::f32::consts::TAU * v;
        let around_direction = Vec2::new(around.cos(), around.sin());
        let cross_cos = cross.cos();
        let position = Vec3::new(
            around_direction.x * (major.x + tube.x * cross_cos),
            around_direction.y * (major.y + tube.y * cross_cos),
            tube.z * cross.sin(),
        );
        let normal = Vec3::new(
            around_direction.x * cross_cos / tube.x.max(f32::EPSILON),
            around_direction.y * cross_cos / tube.y.max(f32::EPSILON),
            cross.sin() / tube.z.max(f32::EPSILON),
        )
        .normalize_or_zero();
        (position, normal)
    })
}

pub(crate) fn capsule(size: Vec3, smoothness: u32) -> Result<ObjMesh, Shape3dError> {
    let slices = BASE_RADIAL_SEGMENTS + smoothness * RADIAL_SEGMENTS_PER_SMOOTHNESS;
    let hemisphere_segments =
        BASE_VERTICAL_SEGMENTS + smoothness * VERTICAL_SEGMENTS_PER_SMOOTHNESS;
    let radius_z = (size.x.min(size.y) * 0.5).min(size.z * 0.5);
    let cylinder_half = size.z * 0.5 - radius_z;
    let radii = Vec3::new(size.x * 0.5, size.y * 0.5, radius_z);
    let mut rings = Vec::new();
    for side in [-1.0_f32, 1.0] {
        for row in 0..=hemisphere_segments {
            let amount = row as f32 / hemisphere_segments as f32;
            let latitude = if side < 0.0 {
                -std::f32::consts::FRAC_PI_2 * (1.0 - amount)
            } else {
                std::f32::consts::FRAC_PI_2 * amount
            };
            rings.push(
                (0..slices)
                    .map(|slice| {
                        let longitude = std::f32::consts::TAU * slice as f32 / slices as f32;
                        let unit = Vec3::new(
                            latitude.cos() * longitude.cos(),
                            latitude.cos() * longitude.sin(),
                            latitude.sin(),
                        );
                        let mut position = unit * radii;
                        position.z += side * cylinder_half;
                        (position, (unit / radii).normalize_or_zero())
                    })
                    .collect::<Vec<_>>(),
            );
        }
    }
    let mut mesh = MeshBuilder::default();
    for rows in rings.windows(2) {
        for slice in 0..slices as usize {
            let next = (slice + 1) % slices as usize;
            let (a, na) = rows[0][slice];
            let (b, nb) = rows[0][next];
            let (c, nc) = rows[1][next];
            let (d, nd) = rows[1][slice];
            mesh.quad([a, b, c, d], [na, nb, nc, nd]);
        }
    }
    mesh.finish()
}

pub(crate) fn cone(
    size: Vec3,
    edge_roundness: f32,
    smoothness: u32,
) -> Result<ObjMesh, Shape3dError> {
    let segments = BASE_RADIAL_SEGMENTS + smoothness * RADIAL_SEGMENTS_PER_SMOOTHNESS;
    let radii = size.truncate() * 0.5;
    let half_height = size.z * 0.5;
    let bevel = edge_roundness
        .max(0.0)
        .min(half_height)
        .min(radii.min_element() * 0.5);
    let mut mesh = MeshBuilder::default();
    let base_scale = if bevel > 0.0 {
        (1.0 - bevel / radii.min_element()).max(0.0)
    } else {
        1.0
    };
    for index in 0..segments {
        let next = (index + 1) % segments;
        let direction = |value: u32| {
            let angle = std::f32::consts::TAU * value as f32 / segments as f32;
            Vec2::new(angle.cos(), angle.sin())
        };
        let a_direction = direction(index);
        let b_direction = direction(next);
        let base_a = (a_direction * radii * base_scale).extend(-half_height);
        let base_b = (b_direction * radii * base_scale).extend(-half_height);
        mesh.triangle(
            [Vec3::new(0.0, 0.0, -half_height), base_b, base_a],
            [-Vec3::Z; 3],
        );
        let side_z = -half_height + bevel;
        let side_a = (a_direction * radii).extend(side_z);
        let side_b = (b_direction * radii).extend(side_z);
        if bevel > 0.0 {
            for step in 0..smoothness {
                let angles = [step, step + 1]
                    .map(|value| value as f32 / smoothness as f32 * std::f32::consts::FRAC_PI_2);
                let ring = |direction: Vec2, angle: f32| {
                    let scale = base_scale + (1.0 - base_scale) * angle.sin();
                    (direction * radii * scale).extend(-half_height + bevel * (1.0 - angle.cos()))
                };
                let a = ring(a_direction, angles[0]);
                let b = ring(b_direction, angles[0]);
                let c = ring(b_direction, angles[1]);
                let d = ring(a_direction, angles[1]);
                let normal = |direction: Vec2, angle: f32| {
                    Vec3::new(
                        direction.x * angle.sin(),
                        direction.y * angle.sin(),
                        -angle.cos(),
                    )
                    .normalize_or_zero()
                };
                mesh.quad(
                    [a, b, c, d],
                    [
                        normal(a_direction, angles[0]),
                        normal(b_direction, angles[0]),
                        normal(b_direction, angles[1]),
                        normal(a_direction, angles[1]),
                    ],
                );
            }
        }
        let tip = Vec3::new(0.0, 0.0, half_height);
        let normal = |direction: Vec2| {
            Vec3::new(
                direction.x / radii.x,
                direction.y / radii.y,
                radii.min_element() / size.z,
            )
            .normalize_or_zero()
        };
        mesh.triangle(
            [side_a, side_b, tip],
            [
                normal(a_direction),
                normal(b_direction),
                normal((a_direction + b_direction).normalize_or_zero()),
            ],
        );
    }
    mesh.finish()
}

fn parametric(
    slices: u32,
    stacks: u32,
    point: impl Fn(f32, f32) -> (Vec3, Vec3),
) -> Result<ObjMesh, Shape3dError> {
    let mut mesh = MeshBuilder::default();
    for stack in 0..stacks {
        let v = stack as f32 / stacks as f32;
        let next_v = (stack + 1) as f32 / stacks as f32;
        for slice in 0..slices {
            let u = slice as f32 / slices as f32;
            let next_u = (slice + 1) as f32 / slices as f32;
            let (a, na) = point(u, v);
            let (b, nb) = point(next_u, v);
            let (c, nc) = point(next_u, next_v);
            let (d, nd) = point(u, next_v);
            mesh.quad([a, b, c, d], [na, nb, nc, nd]);
        }
    }
    mesh.finish()
}

fn contour_outward(contour: &[Vec2]) -> Option<Vec<Vec2>> {
    let area = signed_area(contour);
    if area.abs() <= f32::EPSILON {
        return None;
    }
    Some(
        (0..contour.len())
            .map(|index| {
                let previous = contour[(index + contour.len() - 1) % contour.len()];
                let current = contour[index];
                let next = contour[(index + 1) % contour.len()];
                let edge_normal = |from: Vec2, to: Vec2| {
                    let edge = (to - from).normalize_or_zero();
                    if area > 0.0 {
                        Vec2::new(edge.y, -edge.x)
                    } else {
                        Vec2::new(-edge.y, edge.x)
                    }
                };
                let a = edge_normal(previous, current);
                let b = edge_normal(current, next);
                let direction = (a + b).normalize_or_zero();
                direction * (1.0 / direction.dot(a).abs().max(0.5)).min(MAX_BEVEL_MITER)
            })
            .collect(),
    )
}

fn edge_normals(contour: &[Vec2]) -> Option<Vec<Vec2>> {
    let area = signed_area(contour);
    (area.abs() > f32::EPSILON).then(|| {
        contour
            .iter()
            .zip(contour.iter().cycle().skip(1))
            .map(|(from, to)| {
                let edge = (*to - *from).normalize_or_zero();
                if area > 0.0 {
                    Vec2::new(edge.y, -edge.x)
                } else {
                    Vec2::new(-edge.y, edge.x)
                }
            })
            .collect()
    })
}

fn signed_area(contour: &[Vec2]) -> f32 {
    contour
        .iter()
        .zip(contour.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum()
}

fn safe_roundness(contours: &[Vec<Vec2>], requested: f32) -> f32 {
    if requested <= 0.0 || bevel_is_valid(contours, requested) {
        return requested;
    }
    let mut low = 0.0;
    let mut high = requested;
    for _ in 0..12 {
        let candidate = (low + high) * 0.5;
        if bevel_is_valid(contours, candidate) {
            low = candidate;
        } else {
            high = candidate;
        }
    }
    low * 0.95
}

fn bevel_is_valid(contours: &[Vec<Vec2>], roundness: f32) -> bool {
    let rings = contours
        .iter()
        .map(|contour| {
            let outward = contour_outward(contour)?;
            let ring = contour
                .iter()
                .zip(outward)
                .map(|(point, normal)| *point + normal * roundness)
                .collect::<Vec<_>>();
            (signed_area(contour) * signed_area(&ring) > 0.0).then_some(ring)
        })
        .collect::<Option<Vec<_>>>();
    let Some(rings) = rings else {
        return false;
    };
    for (contour_index, contour) in rings.iter().enumerate() {
        for edge in 0..contour.len() {
            for other in (edge + 1)..contour.len() {
                let edge_next = (edge + 1) % contour.len();
                let other_next = (other + 1) % contour.len();
                if other == edge_next || other_next == edge {
                    continue;
                }
                if segments_cross(
                    contour[edge],
                    contour[edge_next],
                    contour[other],
                    contour[other_next],
                ) {
                    return false;
                }
            }
        }
        for other in &rings[(contour_index + 1)..] {
            for edge in 0..contour.len() {
                for other_edge in 0..other.len() {
                    if segments_cross(
                        contour[edge],
                        contour[(edge + 1) % contour.len()],
                        other[other_edge],
                        other[(other_edge + 1) % other.len()],
                    ) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let side = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    side(a, b, c) * side(a, b, d) < 0.0 && side(c, d, a) * side(c, d, b) < 0.0
}

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 4]>,
    normals: Vec<[f32; 4]>,
    triangles: Vec<[u32; 4]>,
    face_normals: Vec<[f32; 4]>,
}

impl MeshBuilder {
    fn cap(&mut self, points: [Point; 3], z: f32, front: bool) {
        let mut positions = points.map(|point| Vec3::new(point.x, point.y, z));
        if (((positions[1] - positions[0]).cross(positions[2] - positions[0])).z > 0.0) != front {
            positions.swap(1, 2);
        }
        self.triangle(positions, [Vec3::Z * if front { 1.0 } else { -1.0 }; 3]);
    }

    fn extrude_contour(
        &mut self,
        contour: &[Vec2],
        half_depth: f32,
        roundness: f32,
        smoothness: u32,
    ) {
        let Some(outward) = contour_outward(contour) else {
            return;
        };
        let Some(edges) = edge_normals(contour) else {
            return;
        };
        let vertex_normal = |index: usize, face_edge: usize| {
            let previous = edges[(index + edges.len() - 1) % edges.len()];
            let next = edges[index];
            if previous.dot(next) >= SMOOTH_NORMAL_DOT_THRESHOLD {
                (previous + next).normalize_or_zero()
            } else {
                edges[face_edge]
            }
        };
        let bevel_segments = if roundness > 0.0 { smoothness } else { 0 };
        let side_z = half_depth - roundness;
        for side in [1.0_f32, -1.0] {
            for segment in 0..bevel_segments {
                let angles = [segment, segment + 1].map(|value| {
                    value as f32 / bevel_segments as f32 * std::f32::consts::FRAC_PI_2
                });
                for index in 0..contour.len() {
                    let next = (index + 1) % contour.len();
                    let normal_a = vertex_normal(index, index);
                    let normal_b = vertex_normal(next, index);
                    let vertex = |position: Vec2, offset: Vec2, angle: f32| {
                        (position + offset * roundness * angle.sin())
                            .extend(side * (half_depth - roundness * (1.0 - angle.cos())))
                    };
                    let normal = |value: Vec2, angle: f32| {
                        Vec3::new(
                            value.x * angle.sin(),
                            value.y * angle.sin(),
                            side * angle.cos(),
                        )
                        .normalize_or_zero()
                    };
                    self.oriented_quad(
                        [
                            vertex(contour[index], outward[index], angles[0]),
                            vertex(contour[next], outward[next], angles[0]),
                            vertex(contour[next], outward[next], angles[1]),
                            vertex(contour[index], outward[index], angles[1]),
                        ],
                        [
                            normal(normal_a, angles[0]),
                            normal(normal_b, angles[0]),
                            normal(normal_b, angles[1]),
                            normal(normal_a, angles[1]),
                        ],
                        edges[index],
                    );
                }
            }
        }
        for index in 0..contour.len() {
            let next = (index + 1) % contour.len();
            let a2 = contour[index] + outward[index] * roundness;
            let b2 = contour[next] + outward[next] * roundness;
            let normal_a = vertex_normal(index, index).extend(0.0);
            let normal_b = vertex_normal(next, index).extend(0.0);
            self.oriented_quad(
                [
                    a2.extend(side_z),
                    b2.extend(side_z),
                    b2.extend(-side_z),
                    a2.extend(-side_z),
                ],
                [normal_a, normal_b, normal_b, normal_a],
                edges[index],
            );
        }
    }

    fn oriented_quad(&mut self, positions: [Vec3; 4], normals: [Vec3; 4], outward: Vec2) {
        let geometric = (positions[1] - positions[0]).cross(positions[2] - positions[0]);
        if geometric.truncate().dot(outward) >= 0.0 {
            self.quad(positions, normals);
        } else {
            self.triangle(
                [positions[0], positions[2], positions[1]],
                [normals[0], normals[2], normals[1]],
            );
            self.triangle(
                [positions[0], positions[3], positions[2]],
                [normals[0], normals[3], normals[2]],
            );
        }
    }

    fn quad(&mut self, positions: [Vec3; 4], normals: [Vec3; 4]) {
        self.triangle(
            [positions[0], positions[1], positions[2]],
            [normals[0], normals[1], normals[2]],
        );
        self.triangle(
            [positions[0], positions[2], positions[3]],
            [normals[0], normals[2], normals[3]],
        );
    }

    fn triangle(&mut self, positions: [Vec3; 3], normals: [Vec3; 3]) {
        let face = (positions[1] - positions[0])
            .cross(positions[2] - positions[0])
            .normalize_or_zero();
        if face == Vec3::ZERO {
            return;
        }
        let Ok(first) = u32::try_from(self.positions.len()) else {
            return;
        };
        self.positions
            .extend(positions.map(|position| position.extend(1.0).to_array()));
        self.normals
            .extend(normals.map(|normal| normal.normalize_or_zero().extend(0.0).to_array()));
        self.triangles.push([first, first + 1, first + 2, 0]);
        self.face_normals.push(face.extend(0.0).to_array());
    }

    fn finish(self) -> Result<ObjMesh, Shape3dError> {
        if self.positions.is_empty() {
            return Ok(empty_mesh());
        }
        let first = position3(self.positions[0]);
        let (minimum, maximum) = self
            .positions
            .iter()
            .skip(1)
            .map(|position| position3(*position))
            .fold((first, first), |(minimum, maximum), position| {
                (minimum.min(position), maximum.max(position))
            });
        let center = (minimum + maximum) * 0.5;
        let radius = self
            .positions
            .iter()
            .map(|position| (position3(*position) - center).length())
            .fold(0.0, f32::max)
            .max(f32::EPSILON);
        let vertex_count = self.positions.len();
        let triangle_count = self.triangles.len();
        Ok(ObjMesh {
            positions: self.positions,
            normals: self.normals,
            tangents: vec![[1.0, 0.0, 0.0, 1.0]; vertex_count],
            tex_coords_0: vec![[0.0; 4]; vertex_count],
            tex_coords_1: vec![[0.0; 4]; vertex_count],
            colors: vec![shrimply_property_model::Color::WHITE; vertex_count],
            materials: vec![MeshMaterial::default(); triangle_count],
            texture_atlas: TextureAtlas::default(),
            face_normals: self.face_normals,
            triangles: self.triangles,
            source_center: center,
            source_radius: radius,
        })
    }
}

fn position3(position: [f32; 4]) -> Vec3 {
    Vec3::new(position[0], position[1], position[2])
}

fn empty_mesh() -> ObjMesh {
    ObjMesh {
        positions: Vec::new(),
        normals: Vec::new(),
        tangents: Vec::new(),
        tex_coords_0: Vec::new(),
        tex_coords_1: Vec::new(),
        colors: Vec::new(),
        materials: Vec::new(),
        texture_atlas: TextureAtlas::default(),
        face_normals: Vec::new(),
        triangles: Vec::new(),
        source_center: Vec3::ZERO,
        source_radius: 1.0,
    }
}
