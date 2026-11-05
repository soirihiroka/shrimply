use super::Text3dError;
use glam::Vec2;
use hashbrown::HashMap;
use lyon_tessellation::{VertexBuffers, math::Point};

pub(crate) fn boundary_contours(
    fill: &VertexBuffers<Point, u32>,
) -> Result<Vec<Vec<Point>>, Text3dError> {
    let mut boundary = HashMap::new();
    for triangle in fill.indices.chunks_exact(3) {
        let mut indices = [triangle[0], triangle[1], triangle[2]];
        let [a, b, c] = indices.map(|index| fill.vertices[index as usize]);
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        // Lyon may emit zero-area triangles at coincident curve endpoints.
        // MeshBuilder also omits these faces, so they cannot define its boundary.
        if area == 0.0 {
            continue;
        }
        if area < 0.0 {
            indices.swap(1, 2);
        }
        for (from, to) in [
            (indices[0], indices[1]),
            (indices[1], indices[2]),
            (indices[2], indices[0]),
        ] {
            if boundary.remove(&(to, from)).is_none() {
                boundary.insert((from, to), ());
            }
        }
    }

    let mut outgoing = HashMap::<u32, Vec<u32>>::new();
    let mut balance = HashMap::<u32, i32>::new();
    for ((from, to), ()) in boundary {
        outgoing.entry(from).or_default().push(to);
        *balance.entry(from).or_default() += 1;
        *balance.entry(to).or_default() -= 1;
    }
    if balance.values().any(|&value| value != 0) {
        return Err(Text3dError(
            "3D text outline contains an open boundary".into(),
        ));
    }
    let mut contours = Vec::new();
    while let Some((&start, _)) = outgoing.iter().find(|(_, next)| !next.is_empty()) {
        let mut contour = vec![fill.vertices[start as usize]];
        let mut previous = start;
        let mut current = outgoing
            .get_mut(&start)
            .expect("boundary start")
            .pop()
            .expect("nonempty boundary");
        while current != start {
            let position = fill.vertices[current as usize];
            contour.push(position);
            let Some(next) = outgoing.get_mut(&current).filter(|next| !next.is_empty()) else {
                return Err(Text3dError(
                    "3D text outline contains an open boundary".to_string(),
                ));
            };
            let from = fill.vertices[previous as usize];
            let incoming = Vec2::new(from.x - position.x, from.y - position.y);
            // At touching contours, follow the first clockwise edge from the
            // reversed incoming edge. The filled face stays on the left.
            let turn = |index: u32| {
                let point = fill.vertices[index as usize];
                let direction = Vec2::new(point.x - position.x, point.y - position.y);
                (-incoming.angle_to(direction)).rem_euclid(std::f32::consts::TAU)
            };
            let index = (0..next.len())
                .min_by(|&a, &b| turn(next[a]).total_cmp(&turn(next[b])))
                .expect("nonempty boundary");
            previous = current;
            current = next.swap_remove(index);
        }
        if contour.len() >= 3 {
            contours.push(contour);
        }
    }
    Ok(contours)
}

const MAX_BEVEL_MITER: f32 = 2.0;

pub(crate) fn contour_outward(contour: &[Point]) -> Option<Vec<Vec2>> {
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
                let edge_normal = |from: Point, to: Point| {
                    let edge = Vec2::new(to.x - from.x, to.y - from.y).normalize_or_zero();
                    // Extracted boundaries keep the filled region on the left.
                    // The right normal points into empty space, including holes.
                    Vec2::new(edge.y, -edge.x)
                };
                let a = edge_normal(previous, current);
                let b = edge_normal(current, next);
                let direction = (a + b).normalize_or_zero();
                direction * (1.0 / direction.dot(a).abs().max(0.5)).min(MAX_BEVEL_MITER)
            })
            .collect(),
    )
}

pub(crate) fn signed_area(contour: &[Point]) -> f32 {
    contour
        .iter()
        .zip(contour.iter().cycle().skip(1))
        .map(|(a, b)| a.x * b.y - b.x * a.y)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyon_tessellation::math::point;

    #[test]
    fn touching_regions_have_separate_closed_boundaries() {
        let fill = VertexBuffers {
            vertices: vec![
                point(0.0, 0.0),
                point(1.0, 0.0),
                point(1.0, 1.0),
                point(0.0, 1.0),
                point(2.0, 1.0),
                point(2.0, 2.0),
                point(1.0, 2.0),
            ],
            indices: vec![0, 1, 2, 0, 2, 3, 2, 4, 5, 2, 5, 6],
        };
        let contours = boundary_contours(&fill).expect("closed touching regions");
        assert_eq!(contours.len(), 2);
        for contour in contours {
            assert_eq!(contour.len(), 4);
            assert_eq!(signed_area(&contour), 2.0);
        }
    }

    #[test]
    fn degenerate_faces_do_not_change_the_solid_boundary() {
        let fill = VertexBuffers {
            vertices: vec![point(0.0, 0.0), point(1.0, 0.0), point(0.0, 1.0)],
            indices: vec![0, 1, 2, 0, 0, 1],
        };
        let contours = boundary_contours(&fill).expect("closed triangle");
        assert_eq!(contours.len(), 1);
        assert_eq!(contours[0].len(), 3);
    }
    #[test]
    fn bevel_normals_point_away_from_the_filled_region_on_both_sides_of_a_hole() {
        let fill = VertexBuffers {
            vertices: vec![
                point(-2.0, -2.0),
                point(2.0, -2.0),
                point(2.0, 2.0),
                point(-2.0, 2.0),
                point(-1.0, -1.0),
                point(-1.0, 1.0),
                point(1.0, 1.0),
                point(1.0, -1.0),
            ],
            indices: vec![
                0, 1, 7, 0, 7, 4, 1, 2, 6, 1, 6, 7, 2, 3, 5, 2, 5, 6, 3, 0, 4, 3, 4, 5,
            ],
        };
        let contours = boundary_contours(&fill).expect("closed square ring");
        assert_eq!(contours.len(), 2);
        assert!(contours.iter().any(|contour| signed_area(contour) < 0.0));
        for contour in contours {
            let sign = signed_area(&contour).signum();
            let normals = contour_outward(&contour).expect("nondegenerate contour");
            for (point, normal) in contour.iter().zip(normals) {
                assert!(normal.dot(Vec2::new(point.x, point.y) * sign) > 0.0);
            }
        }
    }
}
