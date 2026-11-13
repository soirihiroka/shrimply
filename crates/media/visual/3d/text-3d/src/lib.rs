use std::fmt;

use glam::{Vec2, Vec3};
use hashbrown::HashSet;
use lyon_tessellation::{
    FillOptions, FillRule, FillTessellator, VertexBuffers,
    geometry_builder::{BuffersBuilder, Positions},
    math::{Point, point},
    path::Path as LyonPath,
};
use pango::Alignment;
use serde::{Deserialize, Deserializer, Serialize};
use shrimply_fonts::native as fonts;
pub use shrimply_fonts::native::register_application_font;
use shrimply_property_model::{
    DEFAULT_TEXT_FONT_FAMILY, FontFamily, FontVariation, TextDirection, TextFontStyle,
    TextHorizontalAlign, VerticalAlign,
    modifier_model::{
        KeyframeSpan, ModifierModel, combine, ensure_timeline_value_ids, timeline_value_span,
    },
    timeline_value::TimelineValue,
};
use shrimply_scene_3d::{MeshMaterial, ObjMesh, PbrMaterial, TextureAtlas, Transform3d};
use uuid::Uuid;

mod math;
use math::{contour_outward, signed_area};

pub const MIN_SMOOTHNESS: f32 = 1.0;
pub const MAX_SMOOTHNESS: f32 = 12.0;
pub const DEFAULT_SMOOTHNESS: f32 = 4.0;
pub const DEFAULT_FONT_SIZE: f32 = 1.0;
pub const DEFAULT_DEPTH: f32 = 0.2;
pub const DEFAULT_ROUNDNESS: f32 = 0.03;

const OUTLINE_TOLERANCE_EM: f32 = 0.01;
const LAYOUT_EM_SIZE: f64 = 1024.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Text3dModifier {
    pub text: TimelineValue<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_families: Vec<FontFamily>,
    #[serde(default)]
    pub font_style: TextFontStyle,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_variations: Vec<FontVariation>,
    pub font_weight: TimelineValue<f32>,
    pub h_align: TextHorizontalAlign,
    pub v_align: VerticalAlign,
    pub direction: TextDirection,
    pub font_size: TimelineValue<f32>,
    pub transform: Transform3d,
    pub depth: TimelineValue<f32>,
    pub roundness: TimelineValue<f32>,
    #[serde(
        default = "default_smoothness",
        deserialize_with = "deserialize_smoothness"
    )]
    pub smoothness: TimelineValue<f32>,
    pub material: PbrMaterial,
}

impl Default for Text3dModifier {
    fn default() -> Self {
        Self {
            text: TimelineValue::new_const("Text".to_string()),
            font_families: vec![FontFamily::GoogleFonts {
                name: DEFAULT_TEXT_FONT_FAMILY.to_string(),
            }],
            font_style: TextFontStyle::Normal,
            font_variations: Vec::new(),
            font_weight: TimelineValue::new_const(400.0),
            h_align: TextHorizontalAlign::Center,
            v_align: VerticalAlign::Middle,
            direction: TextDirection::Horizontal,
            font_size: TimelineValue::new_const(DEFAULT_FONT_SIZE),
            transform: Transform3d::default(),
            depth: TimelineValue::new_const(DEFAULT_DEPTH),
            roundness: TimelineValue::new_const(DEFAULT_ROUNDNESS),
            smoothness: default_smoothness(),
            material: PbrMaterial::default(),
        }
    }
}

impl ModifierModel for Text3dModifier {
    fn display_name(&self) -> &'static str {
        "3D Text"
    }

    fn keywords(&self) -> &'static [&'static str] {
        &["extruded text", "type", "typography"]
    }

    fn ensure_ids(&mut self, seen: &mut HashSet<Uuid>) {
        ensure_timeline_value_ids(&mut self.text, seen);
        for value in [
            &mut self.font_weight,
            &mut self.font_size,
            &mut self.depth,
            &mut self.roundness,
            &mut self.smoothness,
        ] {
            ensure_timeline_value_ids(value, seen);
        }
        for value in [
            &mut self.transform.position,
            &mut self.transform.anchor,
            &mut self.transform.rotation_degrees,
            &mut self.transform.scale,
        ] {
            ensure_timeline_value_ids(value, seen);
        }
        ensure_timeline_value_ids(&mut self.transform.rotation_order, seen);
        for value in shrimply_scene_3d::material_numbers_mut(&mut self.material) {
            ensure_timeline_value_ids(value, seen);
        }
        for value in shrimply_scene_3d::material_colors_mut(&mut self.material) {
            ensure_timeline_value_ids(value, seen);
        }
    }

    fn keyframe_span(&self) -> KeyframeSpan {
        combine(
            [
                timeline_value_span(&self.text),
                timeline_value_span(&self.font_weight),
                timeline_value_span(&self.font_size),
                timeline_value_span(&self.depth),
                timeline_value_span(&self.roundness),
                timeline_value_span(&self.smoothness),
                timeline_value_span(&self.transform.position),
                timeline_value_span(&self.transform.anchor),
                timeline_value_span(&self.transform.rotation_degrees),
                timeline_value_span(&self.transform.rotation_order),
                timeline_value_span(&self.transform.scale),
            ]
            .into_iter()
            .chain(
                shrimply_scene_3d::material_numbers(&self.material)
                    .into_iter()
                    .map(timeline_value_span),
            )
            .chain(
                shrimply_scene_3d::material_colors(&self.material)
                    .into_iter()
                    .map(timeline_value_span),
            ),
        )
    }

    fn number(&self, id: Uuid) -> Option<&TimelineValue<f32>> {
        [
            &self.font_weight,
            &self.font_size,
            &self.depth,
            &self.roundness,
            &self.smoothness,
        ]
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers(&self.material))
        .find(|value| value.id == id)
    }

    fn number_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<f32>> {
        [
            &mut self.font_weight,
            &mut self.font_size,
            &mut self.depth,
            &mut self.roundness,
            &mut self.smoothness,
        ]
        .into_iter()
        .chain(shrimply_scene_3d::material_numbers_mut(&mut self.material))
        .find(|value| value.id == id)
    }

    fn number3(&self, id: Uuid) -> Option<&TimelineValue<Vec3>> {
        [
            &self.transform.position,
            &self.transform.anchor,
            &self.transform.rotation_degrees,
            &self.transform.scale,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn number3_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<Vec3>> {
        [
            &mut self.transform.position,
            &mut self.transform.anchor,
            &mut self.transform.rotation_degrees,
            &mut self.transform.scale,
        ]
        .into_iter()
        .find(|value| value.id == id)
    }

    fn color_mut(
        &mut self,
        id: Uuid,
    ) -> Option<&mut TimelineValue<shrimply_property_model::Color<u8>>> {
        shrimply_scene_3d::material_colors_mut(&mut self.material)
            .into_iter()
            .find(|value| value.id == id)
    }

    fn text(&self, id: Uuid) -> Option<&TimelineValue<String>> {
        (self.text.id == id).then_some(&self.text)
    }

    fn text_mut(&mut self, id: Uuid) -> Option<&mut TimelineValue<String>> {
        (self.text.id == id).then_some(&mut self.text)
    }
}

fn default_smoothness() -> TimelineValue<f32> {
    TimelineValue::new_const(DEFAULT_SMOOTHNESS)
}

fn deserialize_smoothness<'de, D>(deserializer: D) -> Result<TimelineValue<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Timeline(TimelineValue<f32>),
        Legacy(f32),
    }
    Ok(match Value::deserialize(deserializer)? {
        Value::Timeline(value) => value,
        Value::Legacy(value) => TimelineValue::new_const(value),
    })
}

pub struct Geometry<'a> {
    pub text: &'a str,
    pub font_families: &'a [FontFamily],
    pub font_style: TextFontStyle,
    pub font_variations: &'a [FontVariation],
    pub font_weight: f32,
    pub h_align: TextHorizontalAlign,
    pub v_align: VerticalAlign,
    pub direction: TextDirection,
    pub font_size: f32,
    pub depth: f32,
    pub roundness: f32,
    pub smoothness: f32,
}

#[derive(Debug)]
pub struct Text3dError(String);

impl fmt::Display for Text3dError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Text3dError {}

pub fn generate_mesh(geometry: &Geometry<'_>) -> Result<ObjMesh, Text3dError> {
    if geometry.text.trim().is_empty() {
        return Ok(empty_mesh());
    }
    if !geometry.smoothness.is_finite() {
        return Err(Text3dError("3D text smoothness is not finite".to_string()));
    }
    let font_size = geometry.font_size.max(f32::EPSILON);
    let depth = geometry.depth.max(f32::EPSILON);
    let smoothness = geometry
        .smoothness
        .round()
        .clamp(MIN_SMOOTHNESS, MAX_SMOOTHNESS) as u32;
    let roundness = geometry.roundness.clamp(0.0, depth * 0.5);
    if !font_size.is_finite() || !depth.is_finite() || !roundness.is_finite() {
        return Err(Text3dError("3D text geometry is not finite".to_string()));
    }

    let path = outline(geometry)?;
    let tolerance = OUTLINE_TOLERANCE_EM / smoothness as f32;
    let mut fill = VertexBuffers::<Point, u32>::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::default()
                .with_fill_rule(FillRule::NonZero)
                .with_tolerance(tolerance),
            &mut BuffersBuilder::new(&mut fill, Positions),
        )
        .map_err(|error| Text3dError(format!("tessellate 3D text caps: {error:?}")))?;
    let contours = math::boundary_contours(&fill)?;
    let roundness = safe_roundness(&contours, roundness, font_size);
    let mut mesh = MeshBuilder::default();
    let half_depth = depth * 0.5;

    for triangle in fill.indices.chunks_exact(3) {
        let points = [
            fill.vertices[triangle[0] as usize],
            fill.vertices[triangle[1] as usize],
            fill.vertices[triangle[2] as usize],
        ];
        mesh.cap(points, half_depth, true, font_size);
        mesh.cap(points, -half_depth, false, font_size);
    }

    for contour in contours {
        mesh.extrude_contour(&contour, half_depth, roundness, smoothness, font_size)?;
    }
    mesh.finish()
}

fn outline(geometry: &Geometry<'_>) -> Result<LyonPath, Text3dError> {
    let native = fonts::shaping::layout(
        &geometry.font_families.iter().map(|font|font.name().to_owned()).collect::<Vec<_>>().into(),
        geometry.text,
        &fonts::shaping::Options {
            weight: geometry.font_weight.round().clamp(1.,1000.) as i32,
            style:match geometry.font_style {
                TextFontStyle::Normal=>pango::Style::Normal,
                TextFontStyle::Italic=>pango::Style::Italic,
                TextFontStyle::Oblique=>pango::Style::Oblique,
            },
            variations:geometry.font_variations.iter().filter(|variation| variation.value.is_finite() && variation.axis.len()==4 && variation.axis.bytes().all(|byte|byte.is_ascii_graphic()) && variation.axis!="wght" && variation.axis!="ital")
                .map(|variation|(variation.axis.clone(),variation.value)).collect(),
            vertical:geometry.direction==TextDirection::Vertical,
            alignment:match geometry.direction {
                TextDirection::Horizontal=>match geometry.h_align {
                    TextHorizontalAlign::Left|TextHorizontalAlign::Fill=>Alignment::Left,
                    TextHorizontalAlign::Center=>Alignment::Center,
                    TextHorizontalAlign::Right=>Alignment::Right,
                },
                TextDirection::Vertical=>match geometry.v_align {
                    VerticalAlign::Top=>Alignment::Left,
                    VerticalAlign::Middle=>Alignment::Center,
                    VerticalAlign::Bottom=>Alignment::Right,
                },
            },
            justify:geometry.direction==TextDirection::Horizontal && geometry.h_align==TextHorizontalAlign::Fill,
            ..fonts::shaping::Options::new(LAYOUT_EM_SIZE)
        },
    ).map_err(Text3dError)?;
    let shaped=&native.shaped;
    let min=Vec2::new((shaped.bounds[0]-shaped.origin[0]) as f32,(shaped.bounds[1]-shaped.origin[1]) as f32);
    let size=Vec2::new((shaped.bounds[2]-shaped.bounds[0]) as f32,(shaped.bounds[3]-shaped.bounds[1]) as f32);
    let anchor = Vec2::new(
        match geometry.h_align {
            TextHorizontalAlign::Left => 0.0,
            TextHorizontalAlign::Center | TextHorizontalAlign::Fill => size.x * 0.5,
            TextHorizontalAlign::Right => size.x,
        },
        match geometry.v_align {
            VerticalAlign::Top => 0.0,
            VerticalAlign::Middle => size.y * 0.5,
            VerticalAlign::Bottom => size.y,
        },
    );
    let convert = |[x, y]: [f64; 2]| {
        point(
            ((x-shaped.origin[0]) as f32 - min.x - anchor.x) / LAYOUT_EM_SIZE as f32,
            -((y-shaped.origin[1]) as f32 - min.y - anchor.y) / LAYOUT_EM_SIZE as f32,
        )
    };
    let mut output = LyonPath::builder();
    let mut open = false;
    for segment in shaped.outline.iter() {
        match segment {
            shrimply_fonts::TextSegment::Move(to) => {
                if open {
                    output.end(false);
                }
                output.begin(convert(*to));
                open = true;
            }
            shrimply_fonts::TextSegment::Line(to) => {
                output.line_to(convert(*to));
            }
            shrimply_fonts::TextSegment::Curve(a, b, to) => {
                output.cubic_bezier_to(convert(*a), convert(*b), convert(*to));
            }
            shrimply_fonts::TextSegment::Close => {
                output.end(true);
                open = false;
            }
        }
    }
    if open {
        output.end(false);
    }
    Ok(output.build())
}

fn safe_roundness(contours: &[Vec<Point>], requested: f32, scale: f32) -> f32 {
    if requested <= 0.0 || bevel_is_valid(contours, requested, scale) {
        return requested;
    }
    let mut low = 0.0;
    let mut high = requested;
    for _ in 0..12 {
        let candidate = (low + high) * 0.5;
        if bevel_is_valid(contours, candidate, scale) {
            low = candidate;
        } else {
            high = candidate;
        }
    }
    low * 0.95
}

fn bevel_is_valid(contours: &[Vec<Point>], roundness: f32, scale: f32) -> bool {
    let rings = contours
        .iter()
        .map(|contour| {
            let outward = contour_outward(contour)?;
            let ring = contour
                .iter()
                .zip(outward)
                .map(|(point, normal)| Vec2::new(point.x, point.y) * scale + normal * roundness)
                .collect::<Vec<_>>();
            let original_area = signed_area(contour);
            let ring_area = ring
                .iter()
                .zip(ring.iter().cycle().skip(1))
                .map(|(a, b)| a.perp_dot(*b))
                .sum::<f32>();
            (original_area * ring_area > 0.0).then_some(ring)
        })
        .collect::<Option<Vec<_>>>();
    let Some(rings) = rings else {
        return false;
    };

    for (contour_index, contour) in rings.iter().enumerate() {
        for edge_index in 0..contour.len() {
            let edge_next = (edge_index + 1) % contour.len();
            for other_index in (edge_index + 1)..contour.len() {
                let other_next = (other_index + 1) % contour.len();
                if other_index == edge_next || other_next == edge_index {
                    continue;
                }
                if segments_cross(
                    contour[edge_index],
                    contour[edge_next],
                    contour[other_index],
                    contour[other_next],
                ) {
                    return false;
                }
            }
        }
        for other in &rings[(contour_index + 1)..] {
            for edge_index in 0..contour.len() {
                let edge_next = (edge_index + 1) % contour.len();
                for other_index in 0..other.len() {
                    if segments_cross(
                        contour[edge_index],
                        contour[edge_next],
                        other[other_index],
                        other[(other_index + 1) % other.len()],
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
    let ab_c = side(a, b, c);
    let ab_d = side(a, b, d);
    let cd_a = side(c, d, a);
    let cd_b = side(c, d, b);
    ab_c * ab_d < 0.0 && cd_a * cd_b < 0.0
}

#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 4]>,
    normals: Vec<[f32; 4]>,
    triangles: Vec<[u32; 4]>,
    face_normals: Vec<[f32; 4]>,
}

impl MeshBuilder {
    fn cap(&mut self, points: [Point; 3], z: f32, front: bool, scale: f32) {
        let mut positions = points.map(|p| Vec3::new(p.x * scale, p.y * scale, z));
        if ((positions[1] - positions[0])
            .cross(positions[2] - positions[0])
            .z
            > 0.0)
            != front
        {
            positions.swap(1, 2);
        }
        self.triangle(positions, [Vec3::Z * if front { 1.0 } else { -1.0 }; 3]);
    }

    fn extrude_contour(
        &mut self,
        contour: &[Point],
        half_depth: f32,
        roundness: f32,
        smoothness: u32,
        scale: f32,
    ) -> Result<(), Text3dError> {
        let Some(outward) = contour_outward(contour) else {
            return Ok(());
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
                    let vertex = |point: Point, normal: Vec2, angle: f32| {
                        Vec3::new(point.x, point.y, 0.0) * Vec3::new(scale, scale, 0.0)
                            + Vec3::new(normal.x, normal.y, 0.0) * roundness * angle.sin()
                            + Vec3::Z * side * (half_depth - roundness * (1.0 - angle.cos()))
                    };
                    let normal = |value: Vec2, angle: f32| {
                        Vec3::new(
                            value.x * angle.sin(),
                            value.y * angle.sin(),
                            side * angle.cos(),
                        )
                        .normalize_or_zero()
                    };
                    let a = vertex(contour[index], outward[index], angles[0]);
                    let b = vertex(contour[next], outward[next], angles[0]);
                    let c = vertex(contour[next], outward[next], angles[1]);
                    let d = vertex(contour[index], outward[index], angles[1]);
                    let na = normal(outward[index], angles[0]);
                    let nb = normal(outward[next], angles[0]);
                    let nc = normal(outward[next], angles[1]);
                    let nd = normal(outward[index], angles[1]);
                    self.oriented_quad([a, b, c, d], [na, nb, nc, nd], outward[index]);
                }
            }
        }
        for index in 0..contour.len() {
            let next = (index + 1) % contour.len();
            let a2 =
                Vec2::new(contour[index].x, contour[index].y) * scale + outward[index] * roundness;
            let b2 =
                Vec2::new(contour[next].x, contour[next].y) * scale + outward[next] * roundness;
            let a = Vec3::new(a2.x, a2.y, side_z);
            let b = Vec3::new(b2.x, b2.y, side_z);
            let c = Vec3::new(b2.x, b2.y, -side_z);
            let d = Vec3::new(a2.x, a2.y, -side_z);
            let normal_a = outward[index].extend(0.0).normalize_or_zero();
            let normal_b = outward[next].extend(0.0).normalize_or_zero();
            self.oriented_quad(
                [a, b, c, d],
                [normal_a, normal_b, normal_b, normal_a],
                outward[index],
            );
        }
        Ok(())
    }

    fn oriented_quad(&mut self, p: [Vec3; 4], n: [Vec3; 4], outward: Vec2) {
        let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
        if geometric.truncate().dot(outward) >= 0.0 {
            self.triangle([p[0], p[1], p[2]], [n[0], n[1], n[2]]);
            self.triangle([p[0], p[2], p[3]], [n[0], n[2], n[3]]);
        } else {
            self.triangle([p[0], p[2], p[1]], [n[0], n[2], n[1]]);
            self.triangle([p[0], p[3], p[2]], [n[0], n[3], n[2]]);
        }
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
            .extend(normals.map(|normal| normal.extend(0.0).to_array()));
        self.triangles.push([first, first + 1, first + 2, 0]);
        self.face_normals.push(face.extend(0.0).to_array());
    }

    fn finish(self) -> Result<ObjMesh, Text3dError> {
        if self.positions.is_empty() {
            return Ok(empty_mesh());
        }
        let first = Vec3::from_array(self.positions[0][..3].try_into().unwrap());
        let (minimum, maximum) =
            self.positions
                .iter()
                .skip(1)
                .fold((first, first), |(minimum, maximum), position| {
                    let position = Vec3::from_array(position[..3].try_into().unwrap());
                    (minimum.min(position), maximum.max(position))
                });
        let center = (minimum + maximum) * 0.5;
        let radius = self
            .positions
            .iter()
            .map(|position| (Vec3::from_array(position[..3].try_into().unwrap()) - center).length())
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
