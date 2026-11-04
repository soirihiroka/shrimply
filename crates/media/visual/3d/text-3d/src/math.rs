use super::Text3dError;
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
    for ((from, to), ()) in boundary {
        outgoing.entry(from).or_default().push(to);
    }
    if let Some((&vertex, next)) = outgoing.iter().find(|(_, next)| next.len() != 1) {
        return Err(Text3dError(format!(
            "3D text outline contains a non-manifold boundary at {:?}: {next:?}",
            fill.vertices[vertex as usize]
        )));
    }
    let mut contours = Vec::new();
    while let Some((&start, _)) = outgoing.iter().find(|(_, next)| !next.is_empty()) {
        let mut contour = Vec::new();
        let mut current = start;
        loop {
            contour.push(fill.vertices[current as usize]);
            let Some(next) = outgoing.get_mut(&current).and_then(Vec::pop) else {
                return Err(Text3dError(
                    "3D text outline contains an open boundary".to_string(),
                ));
            };
            current = next;
            if current == start {
                break;
            }
        }
        if contour.len() >= 3 {
            contours.push(contour);
        }
    }
    Ok(contours)
}
