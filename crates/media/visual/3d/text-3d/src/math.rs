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
