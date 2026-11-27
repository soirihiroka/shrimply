use std::cmp::Ordering;
use std::collections::BTreeSet;

use glam::{DVec2, Vec2};
use rayon::prelude::*;
use shrimply_math_geometry::Rect;

const RELATIVE_VERTEX_EPSILON: f64 = 1.0e-7;
const MINIMUM_VERTEX_EPSILON: f64 = 1.0e-7;
const PARALLEL_EPSILON: f64 = 1.0e-12;
const PARAMETER_EPSILON: f64 = 1.0e-12;

#[derive(Clone, Debug)]
pub struct Face {
    index: usize,
    loops: Vec<Vec<Vec2>>,
    bounds: Rect,
    area: f32,
    epsilon: f64,
}

impl Face {
    pub fn index(&self) -> usize {
        self.index
    }

    /// The first loop is the outer boundary. Remaining loops are holes.
    /// Consumers should use an even-odd fill rule.
    pub fn loops(&self) -> &[Vec<Vec2>] {
        &self.loops
    }

    pub fn outer(&self) -> &[Vec2] {
        &self.loops[0]
    }

    pub fn holes(&self) -> &[Vec<Vec2>] {
        &self.loops[1..]
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn area(&self) -> f32 {
        self.area
    }

    pub fn contains(&self, point: Vec2) -> bool {
        if !self.bounds.contains(point) {
            return false;
        }

        let point = point.as_dvec2();
        self.loops
            .iter()
            .filter(|boundary| point_in_polygon(point, boundary, self.epsilon))
            .count()
            % 2
            == 1
    }
}

#[derive(Clone, Debug)]
pub struct Topology {
    canvas_size: Vec2,
    faces: Vec<Face>,
}

impl Topology {
    pub fn empty(canvas_size: Vec2) -> Self {
        Self {
            canvas_size,
            faces: Vec::new(),
        }
    }

    pub fn canvas_size(&self) -> Vec2 {
        self.canvas_size
    }

    pub fn faces(&self) -> &[Face] {
        &self.faces
    }

    pub fn face_at(&self, point: Vec2) -> Option<&Face> {
        if !point.is_finite() {
            return None;
        }

        self.faces
            .iter()
            .filter(|face| face.contains(point))
            .min_by(|left, right| {
                left.area
                    .total_cmp(&right.area)
                    .then_with(|| left.index.cmp(&right.index))
            })
    }
}

/// Builds the planar subdivision made by `centerlines` inside the canvas.
///
/// Non-finite points split a polyline into separate runs. Duplicate and
/// zero-length segments are ignored. A non-finite or non-positive canvas
/// produces an empty topology.
pub fn build(centerlines: &[Vec<Vec2>], canvas_size: Vec2, closure_tolerance: f32) -> Topology {
    if !canvas_size.is_finite() || canvas_size.cmple(Vec2::ZERO).any() {
        return Topology::empty(canvas_size);
    }

    let canvas = canvas_size.as_dvec2();
    let epsilon = (canvas.max_element() * RELATIVE_VERTEX_EPSILON).max(MINIMUM_VERTEX_EPSILON);
    let tolerance = if closure_tolerance.is_finite() {
        f64::from(closure_tolerance.max(0.0))
    } else {
        0.0
    };

    let polylines = finite_runs(centerlines, epsilon);
    let mut segments = clipped_segments(&polylines, canvas, epsilon);
    segments.extend(closure_segments(&polylines, canvas, tolerance, epsilon));
    segments.extend(canvas_segments(canvas));
    let (vertices, edges) = split_segments(&segments, epsilon);
    let cycles = walk_cycles(&vertices, &edges, epsilon);
    let faces = assemble_faces(cycles, epsilon);

    Topology { canvas_size, faces }
}

#[derive(Clone, Debug)]
struct Polyline {
    points: Vec<DVec2>,
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    start: DVec2,
    end: DVec2,
}

#[derive(Clone, Copy, Debug)]
struct Endpoint {
    polyline: usize,
    point: usize,
}

#[derive(Clone, Debug)]
struct Cycle {
    points: Vec<Vec2>,
    signed_area: f64,
}

fn finite_runs(centerlines: &[Vec<Vec2>], epsilon: f64) -> Vec<Polyline> {
    let mut runs = Vec::new();

    for centerline in centerlines {
        let mut points = Vec::new();
        for &point in centerline {
            if point.is_finite() {
                let point = point.as_dvec2();
                if points
                    .last()
                    .is_none_or(|previous: &DVec2| previous.distance(point) > epsilon)
                {
                    points.push(point);
                }
            } else if points.len() > 1 {
                runs.push(Polyline {
                    points: std::mem::take(&mut points),
                });
            } else {
                points.clear();
            }
        }
        if points.len() > 1 {
            runs.push(Polyline { points });
        }
    }

    runs
}

fn closure_segments(
    polylines: &[Polyline],
    canvas: DVec2,
    tolerance: f64,
    epsilon: f64,
) -> Vec<Segment> {
    let endpoints: Vec<_> = polylines
        .iter()
        .enumerate()
        .flat_map(|(polyline, line)| {
            [
                Endpoint { polyline, point: 0 },
                Endpoint {
                    polyline,
                    point: line.points.len() - 1,
                },
            ]
        })
        .collect();
    endpoints
        .iter()
        .enumerate()
        .filter_map(|(index, &endpoint)| {
            let point = endpoint_point(polylines, endpoint);
            let target = nearest_snap_target(
                point, index, &endpoints, polylines, canvas, tolerance, epsilon,
            )?;
            (point.distance(target) > epsilon)
                .then_some(Segment {
                    start: point,
                    end: target,
                })
                .and_then(|segment| clip_to_canvas(segment.start, segment.end, canvas))
        })
        .collect()
}

fn nearest_snap_target(
    point: DVec2,
    endpoint_index: usize,
    endpoints: &[Endpoint],
    polylines: &[Polyline],
    canvas: DVec2,
    tolerance: f64,
    epsilon: f64,
) -> Option<DVec2> {
    let mut best: Option<(f64, usize, DVec2)> = None;
    let mut consider = |target: DVec2, order: usize| {
        let distance = point.distance(target);
        if distance <= tolerance
            && best.is_none_or(|current| {
                distance < current.0 - epsilon
                    || ((distance - current.0).abs() <= epsilon && order < current.1)
            })
        {
            best = Some((distance, order, target));
        }
    };

    let clamped = point.clamp(DVec2::ZERO, canvas);
    consider(DVec2::new(0.0, clamped.y), 0);
    consider(DVec2::new(canvas.x, clamped.y), 1);
    consider(DVec2::new(clamped.x, 0.0), 2);
    consider(DVec2::new(clamped.x, canvas.y), 3);

    for (index, &endpoint) in endpoints.iter().enumerate() {
        if index != endpoint_index {
            consider(endpoint_point(polylines, endpoint), 4 + index);
        }
    }

    let is_local_endpoint_segment = |polyline: usize, segment: usize| {
        let endpoint = endpoints[endpoint_index];
        if endpoint.polyline != polyline {
            return false;
        }
        let points = &polylines[polyline].points;
        let arclength = if endpoint.point == 0 {
            points
                .windows(2)
                .take(segment)
                .map(|pair| pair[0].distance(pair[1]))
                .sum::<f64>()
        } else {
            points
                .windows(2)
                .skip(segment + 1)
                .map(|pair| pair[0].distance(pair[1]))
                .sum::<f64>()
        };
        arclength <= tolerance + epsilon
    };
    let mut order = 4 + endpoints.len();
    for (polyline, line) in polylines.iter().enumerate() {
        for (segment, pair) in line.points.windows(2).enumerate() {
            if !is_local_endpoint_segment(polyline, segment) {
                consider(closest_point_on_segment(point, pair[0], pair[1]), order);
            }
            order += 1;
        }
    }

    best.map(|(_, _, target)| target)
}

fn endpoint_point(polylines: &[Polyline], endpoint: Endpoint) -> DVec2 {
    polylines[endpoint.polyline].points[endpoint.point]
}

fn closest_point_on_segment(point: DVec2, start: DVec2, end: DVec2) -> DVec2 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared == 0.0 {
        start
    } else {
        start + segment * ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0)
    }
}

fn clipped_segments(polylines: &[Polyline], canvas: DVec2, epsilon: f64) -> Vec<Segment> {
    polylines
        .iter()
        .flat_map(|line| line.points.windows(2))
        .filter_map(|pair| clip_to_canvas(pair[0], pair[1], canvas))
        .filter(|segment| segment.start.distance(segment.end) > epsilon)
        .collect()
}

fn clip_to_canvas(start: DVec2, end: DVec2, canvas: DVec2) -> Option<Segment> {
    let direction = end - start;
    let mut minimum = 0.0_f64;
    let mut maximum = 1.0_f64;
    for (origin, delta, limit) in [
        (start.x, direction.x, canvas.x),
        (start.y, direction.y, canvas.y),
    ] {
        if delta == 0.0 {
            if origin < 0.0 || origin > limit {
                return None;
            }
            continue;
        }

        let first = (0.0 - origin) / delta;
        let second = (limit - origin) / delta;
        minimum = minimum.max(first.min(second));
        maximum = maximum.min(first.max(second));
        if minimum > maximum {
            return None;
        }
    }

    Some(Segment {
        start: start + direction * minimum,
        end: start + direction * maximum,
    })
}

fn canvas_segments(canvas: DVec2) -> [Segment; 4] {
    let top_left = DVec2::ZERO;
    let top_right = DVec2::new(canvas.x, 0.0);
    let bottom_right = canvas;
    let bottom_left = DVec2::new(0.0, canvas.y);
    [
        Segment {
            start: top_left,
            end: top_right,
        },
        Segment {
            start: top_right,
            end: bottom_right,
        },
        Segment {
            start: bottom_right,
            end: bottom_left,
        },
        Segment {
            start: bottom_left,
            end: top_left,
        },
    ]
}

fn split_segments(segments: &[Segment], epsilon: f64) -> (Vec<DVec2>, Vec<(usize, usize)>) {
    let mut parameters = vec![vec![0.0, 1.0]; segments.len()];
    let intersections: Vec<_> = (0..segments.len())
        .into_par_iter()
        .map(|left| {
            (left + 1..segments.len())
                .filter_map(|right| {
                    let intersections = intersections(segments[left], segments[right], epsilon);
                    (!intersections.is_empty()).then_some((right, intersections))
                })
                .collect::<Vec<_>>()
        })
        .collect();
    for (left, intersections) in intersections.into_iter().enumerate() {
        for (right, intersections) in intersections {
            for (left_parameter, right_parameter) in intersections {
                parameters[left].push(left_parameter);
                parameters[right].push(right_parameter);
            }
        }
    }

    let mut vertices = Vec::new();
    let mut edges = BTreeSet::new();
    for (segment, mut parameters) in segments.iter().zip(parameters) {
        parameters.sort_by(f64::total_cmp);
        parameters.dedup_by(|left, right| (*left - *right).abs() <= PARAMETER_EPSILON);
        for pair in parameters.windows(2) {
            let start = segment.start.lerp(segment.end, pair[0]);
            let end = segment.start.lerp(segment.end, pair[1]);
            if start.distance(end) <= epsilon {
                continue;
            }
            let start = intern_vertex(&mut vertices, start, epsilon);
            let end = intern_vertex(&mut vertices, end, epsilon);
            if start != end {
                edges.insert(if start < end {
                    (start, end)
                } else {
                    (end, start)
                });
            }
        }
    }

    (vertices, edges.into_iter().collect())
}

fn intersections(left: Segment, right: Segment, epsilon: f64) -> Vec<(f64, f64)> {
    let left_direction = left.end - left.start;
    let right_direction = right.end - right.start;
    let offset = right.start - left.start;
    let denominator = cross(left_direction, right_direction);
    let parallel_threshold = left_direction.length() * right_direction.length() * PARALLEL_EPSILON;

    if denominator.abs() > parallel_threshold {
        let left_parameter = cross(offset, right_direction) / denominator;
        let right_parameter = cross(offset, left_direction) / denominator;
        if (-PARAMETER_EPSILON..=1.0 + PARAMETER_EPSILON).contains(&left_parameter)
            && (-PARAMETER_EPSILON..=1.0 + PARAMETER_EPSILON).contains(&right_parameter)
        {
            return vec![(
                left_parameter.clamp(0.0, 1.0),
                right_parameter.clamp(0.0, 1.0),
            )];
        }
        return Vec::new();
    }

    if cross(offset, left_direction).abs() > epsilon * left_direction.length() {
        return Vec::new();
    }

    let mut intersections = Vec::new();
    for left_parameter in [0.0, 1.0] {
        let point = left.start.lerp(left.end, left_parameter);
        if let Some(right_parameter) = parameter_on_segment(point, right, epsilon) {
            intersections.push((left_parameter, right_parameter));
        }
    }
    for right_parameter in [0.0, 1.0] {
        let point = right.start.lerp(right.end, right_parameter);
        if let Some(left_parameter) = parameter_on_segment(point, left, epsilon) {
            intersections.push((left_parameter, right_parameter));
        }
    }
    intersections
}

fn parameter_on_segment(point: DVec2, segment: Segment, epsilon: f64) -> Option<f64> {
    let direction = segment.end - segment.start;
    let parameter = (point - segment.start).dot(direction) / direction.length_squared();
    let projection = segment.start + direction * parameter;
    ((-PARAMETER_EPSILON..=1.0 + PARAMETER_EPSILON).contains(&parameter)
        && projection.distance(point) <= epsilon)
        .then(|| parameter.clamp(0.0, 1.0))
}

fn cross(left: DVec2, right: DVec2) -> f64 {
    left.x * right.y - left.y * right.x
}

fn intern_vertex(vertices: &mut Vec<DVec2>, point: DVec2, epsilon: f64) -> usize {
    if let Some(index) = vertices
        .iter()
        .position(|vertex| vertex.distance(point) <= epsilon)
    {
        index
    } else {
        vertices.push(point);
        vertices.len() - 1
    }
}

#[derive(Clone, Copy, Debug)]
struct HalfEdge {
    from: usize,
    to: usize,
    twin: usize,
}

fn walk_cycles(vertices: &[DVec2], edges: &[(usize, usize)], epsilon: f64) -> Vec<Cycle> {
    let mut half_edges = Vec::with_capacity(edges.len() * 2);
    let mut outgoing = vec![Vec::new(); vertices.len()];
    for &(left, right) in edges {
        let forward = half_edges.len();
        half_edges.push(HalfEdge {
            from: left,
            to: right,
            twin: forward + 1,
        });
        half_edges.push(HalfEdge {
            from: right,
            to: left,
            twin: forward,
        });
        outgoing[left].push(forward);
        outgoing[right].push(forward + 1);
    }

    for (vertex, edges) in outgoing.iter_mut().enumerate() {
        edges.sort_by(|&left, &right| {
            let left_direction = vertices[half_edges[left].to] - vertices[vertex];
            let right_direction = vertices[half_edges[right].to] - vertices[vertex];
            left_direction
                .y
                .atan2(left_direction.x)
                .total_cmp(&right_direction.y.atan2(right_direction.x))
                .then_with(|| half_edges[left].to.cmp(&half_edges[right].to))
        });
    }

    let mut next = vec![0; half_edges.len()];
    for (index, edge) in half_edges.iter().enumerate() {
        let edges = &outgoing[edge.to];
        let twin = edges
            .iter()
            .position(|&candidate| candidate == edge.twin)
            .unwrap();
        next[index] = edges[(twin + edges.len() - 1) % edges.len()];
    }

    let mut visited = vec![false; half_edges.len()];
    let mut cycles = Vec::new();
    for start in 0..half_edges.len() {
        if visited[start] {
            continue;
        }

        let mut current = start;
        let mut points = Vec::new();
        loop {
            if visited[current] {
                break;
            }
            visited[current] = true;
            points.push(vertices[half_edges[current].from].as_vec2());
            current = next[current];
            if current == start {
                break;
            }
        }

        let signed_area = polygon_signed_area(&points);
        if current == start && signed_area.abs() > epsilon * epsilon {
            canonicalize_loop(&mut points);
            cycles.push(Cycle {
                points,
                signed_area,
            });
        }
    }
    cycles
}

fn assemble_faces(cycles: Vec<Cycle>, epsilon: f64) -> Vec<Face> {
    let mut outers: Vec<_> = cycles
        .iter()
        .filter(|cycle| cycle.signed_area > 0.0)
        .cloned()
        .collect();
    let holes: Vec<_> = cycles
        .into_iter()
        .filter(|cycle| cycle.signed_area < 0.0)
        .collect();
    outers.sort_by(|left, right| compare_loops(&left.points, &right.points));

    let mut face_holes = vec![Vec::<Cycle>::new(); outers.len()];
    for hole in holes {
        let point = hole.points[0].as_dvec2();
        let parent = outers
            .iter()
            .enumerate()
            .filter(|(_, outer)| {
                outer.signed_area > -hole.signed_area + epsilon * epsilon
                    && point_in_polygon(point, &outer.points, epsilon)
            })
            .min_by(|(_, left), (_, right)| left.signed_area.total_cmp(&right.signed_area))
            .map(|(index, _)| index);
        if let Some(parent) = parent {
            face_holes[parent].push(hole);
        }
    }

    let mut faces = Vec::with_capacity(outers.len());
    for (outer, mut holes) in outers.into_iter().zip(face_holes) {
        holes.sort_by(|left, right| compare_loops(&left.points, &right.points));
        let mut loops = Vec::with_capacity(holes.len() + 1);
        loops.push(outer.points);
        loops.extend(holes.into_iter().map(|hole| hole.points));
        let bounds = polygon_bounds(&loops[0]);
        let area = (outer.signed_area
            + loops[1..]
                .iter()
                .map(|hole| polygon_signed_area(hole))
                .sum::<f64>())
        .max(0.0) as f32;
        faces.push(Face {
            index: faces.len(),
            loops,
            bounds,
            area,
            epsilon,
        });
    }
    faces
}

fn polygon_signed_area(points: &[Vec2]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| {
            f64::from(left.x) * f64::from(right.y) - f64::from(left.y) * f64::from(right.x)
        })
        .sum::<f64>()
        * 0.5
}

fn canonicalize_loop(points: &mut [Vec2]) {
    if let Some((index, _)) = points.iter().enumerate().min_by(|(_, left), (_, right)| {
        left.x
            .total_cmp(&right.x)
            .then_with(|| left.y.total_cmp(&right.y))
    }) {
        points.rotate_left(index);
    }
}

fn compare_loops(left: &[Vec2], right: &[Vec2]) -> Ordering {
    left.iter()
        .zip(right)
        .find_map(|(left, right)| {
            let ordering = left
                .x
                .total_cmp(&right.x)
                .then_with(|| left.y.total_cmp(&right.y));
            (ordering != Ordering::Equal).then_some(ordering)
        })
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn polygon_bounds(points: &[Vec2]) -> Rect {
    let mut minimum = Vec2::splat(f32::INFINITY);
    let mut maximum = Vec2::splat(f32::NEG_INFINITY);
    for &point in points {
        minimum = minimum.min(point);
        maximum = maximum.max(point);
    }
    Rect::from_min_max(minimum, maximum)
}

fn point_in_polygon(point: DVec2, polygon: &[Vec2], epsilon: f64) -> bool {
    let mut inside = false;
    for (&start, &end) in polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
    {
        let start = start.as_dvec2();
        let end = end.as_dvec2();
        let edge = end - start;
        if closest_point_on_segment(point, start, end).distance(point) <= epsilon {
            return true;
        }
        if (start.y > point.y) != (end.y > point.y)
            && point.x < start.x + edge.x * (point.y - start.y) / edge.y
        {
            inside = !inside;
        }
    }
    inside
}
