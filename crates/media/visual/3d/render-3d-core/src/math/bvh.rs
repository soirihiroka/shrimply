use crate::ObjRenderSession;
use glam::{Mat4, Vec3};

const LEAF_TRIANGLES: usize = 4;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SoftwareBvhNode {
    pub minimum: [f32; 4],
    pub maximum: [f32; 4],
    pub range: [u32; 4],
}

pub struct SoftwareBvh {
    pub nodes: Vec<SoftwareBvhNode>,
    pub triangles: Vec<[u32; 4]>,
}

struct Triangle {
    reference: [u32; 4],
    minimum: Vec3,
    maximum: Vec3,
}

pub fn software_bvh(session: &ObjRenderSession) -> Result<SoftwareBvh, String> {
    let mut triangles = Vec::new();
    for (index, instance) in session.acceleration_instances().iter().enumerate() {
        let geometry = &session.geometries()[instance.geometry_index as usize];
        let transform = Mat4::from_cols_array(&instance.transform);
        for part in 0..geometry.geometry_count as usize {
            for primitive in 0..geometry.primitive_counts[part] {
                let first = geometry.vertex_offsets[part] as usize + primitive as usize * 3;
                let points = session.positions()[first..first + 3].iter()
                    .map(|position| transform.transform_point3(Vec3::from_array(position[..3].try_into().expect("three coordinates"))));
                let (minimum, maximum) = points.fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
                    |(minimum, maximum), point| (minimum.min(point), maximum.max(point)));
                if !minimum.is_finite() || !maximum.is_finite() { return Err("3D geometry contains non-finite positions".into()); }
                triangles.push(Triangle { reference: [u32::try_from(index).map_err(|_| "Too many 3D instances")?, part as u32, primitive, 0], minimum, maximum });
            }
        }
    }
    if triangles.is_empty() { return Err("3D scene has no triangles".into()); }
    let mut result = SoftwareBvh { nodes: Vec::new(), triangles: Vec::new() };
    build(&mut triangles, &mut result)?;
    Ok(result)
}

fn build(triangles: &mut [Triangle], result: &mut SoftwareBvh) -> Result<(), String> {
    let (minimum, maximum) = triangles.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)),
        |(minimum, maximum), triangle| (minimum.min(triangle.minimum), maximum.max(triangle.maximum)));
    let node = result.nodes.len();
    result.nodes.push(SoftwareBvhNode { minimum: minimum.extend(0.0).to_array(), maximum: maximum.extend(0.0).to_array(), range: [0; 4] });
    if triangles.len() <= LEAF_TRIANGLES {
        result.nodes[node].range[0] = u32::try_from(result.triangles.len()).map_err(|_| "Too many BVH triangles")?;
        result.nodes[node].range[1] = triangles.len() as u32;
        result.triangles.extend(triangles.iter().map(|triangle| triangle.reference));
    } else {
        let extent = maximum - minimum;
        let axis = if extent.x >= extent.y && extent.x >= extent.z { 0 } else if extent.y >= extent.z { 1 } else { 2 };
        let middle = triangles.len() / 2;
        triangles.select_nth_unstable_by(middle, |left, right| {
            (left.minimum[axis] + left.maximum[axis]).total_cmp(&(right.minimum[axis] + right.maximum[axis]))
        });
        let (left, right) = triangles.split_at_mut(middle);
        build(left, result)?;
        build(right, result)?;
    }
    result.nodes[node].range[2] = u32::try_from(result.nodes.len()).map_err(|_| "Too many BVH nodes")?;
    Ok(())
}
