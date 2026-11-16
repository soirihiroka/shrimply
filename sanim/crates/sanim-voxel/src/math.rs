//! Bounded voxel-grid ray traversal. Distances are spatial, never timeline values.
mod foliage;
pub(crate) mod instances;
mod optics;

use crate::{CHUNK_EDGE, CHUNK_VOXELS, EMPTY, MaterialId};

// Covers sparse B-tree leaves, branch entries and allocation headers. Counting
// binary-height paths conservatively bounds copies in the higher-fanout tree.
const MAP_ENTRY_BYTES: usize = 512;
pub(crate) fn map_metadata_bytes(entries: usize) -> usize {
    entries.saturating_mul(MAP_ENTRY_BYTES)
}
pub(crate) fn copied_map_metadata_bytes(entries: usize, previous: usize, changed: usize) -> usize {
    let levels = entries.max(previous).checked_ilog2().unwrap_or(0) as usize + 1;
    map_metadata_bytes(entries).min(
        changed
            .saturating_mul(levels)
            .saturating_mul(MAP_ENTRY_BYTES),
    )
}

/// Matches the GPU ray offset, in voxel-cell units.
pub const SURFACE_OFFSET: f64 = 0.0001;

pub fn face_uv(normal: [i32; 3], point: [f64; 3]) -> (usize, [f64; 2]) {
    let [x, y, z] = point.map(|axis| axis - axis.floor());
    match normal {
        [1, 0, 0] => (0, [1.0 - z, 1.0 - y]),
        [-1, 0, 0] => (1, [z, 1.0 - y]),
        [0, 1, 0] => (2, [x, z]),
        [0, -1, 0] => (3, [x, 1.0 - z]),
        [0, 0, 1] => (4, [x, 1.0 - y]),
        [0, 0, -1] => (5, [1.0 - x, 1.0 - y]),
        _ => panic!("voxel surface normal must be one signed axis"),
    }
}

pub(crate) fn validate_dimensions(dimensions: [u32; 3], maximum: u32) -> Result<(), String> {
    for (axis, size) in dimensions.into_iter().enumerate() {
        if size == 0 || size > maximum {
            return Err(format!(
                "voxel dimension {} is {size}; expected 1..={maximum}",
                ["x", "y", "z"][axis]
            ));
        }
    }
    Ok(())
}

pub(crate) fn fill_work(min: [u32; 3], max: [u32; 3]) -> usize {
    if (0..3).any(|axis| min[axis] == max[axis]) {
        return 0;
    }
    let touched = (0..3)
        .map(|axis| (max[axis].div_ceil(CHUNK_EDGE) - min[axis] / CHUNK_EDGE) as usize)
        .product::<usize>();
    let full = (0..3)
        .map(|axis| {
            (max[axis] / CHUNK_EDGE).saturating_sub(min[axis].div_ceil(CHUNK_EDGE)) as usize
        })
        .product::<usize>();
    full + (touched - full) * CHUNK_VOXELS
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct Hit {
    pub position: [u32; 3],
    pub normal: [f64; 3],
    pub distance: f64,
    pub material: MaterialId,
    pub color: u32,
}

/// Trace a normalized-distance ray, including rays beginning inside the volume.
pub fn raycast<V: crate::GridView + ?Sized>(
    volume: &V,
    origin: [f64; 3],
    direction: [f64; 3],
) -> Result<Option<Hit>, String> {
    if !origin.into_iter().chain(direction).all(f64::is_finite) {
        return Err("voxel ray origin and direction must be finite".into());
    }
    let scale = direction.into_iter().map(f64::abs).fold(0.0, f64::max);
    if scale == 0.0 {
        return Err("voxel ray direction must not be zero".into());
    }
    let scaled = direction.map(|axis| axis / scale);
    let length = scaled
        .into_iter()
        .map(|axis| axis * axis)
        .sum::<f64>()
        .sqrt();
    let direction = scaled.map(|axis| axis / length);
    let dimensions = volume.dimensions();
    let mut enter: f64 = 0.0;
    let mut exit = f64::INFINITY;
    let mut normal = [0; 3];
    for axis in 0..3 {
        if direction[axis] == 0.0 {
            if origin[axis] < 0.0 || origin[axis] >= f64::from(dimensions[axis]) {
                return Ok(None);
            }
            continue;
        }
        let low = -origin[axis] / direction[axis];
        let high = (f64::from(dimensions[axis]) - origin[axis]) / direction[axis];
        let near = low.min(high);
        let far = low.max(high);
        if near > enter || (near == enter && normal == [0; 3]) {
            enter = near;
            normal = [0; 3];
            normal[axis] = if direction[axis] > 0.0 { -1 } else { 1 };
        }
        exit = exit.min(far);
    }
    if enter >= exit || exit <= 0.0 {
        return Ok(None);
    }
    let point = std::array::from_fn::<_, 3, _>(|axis| origin[axis] + enter * direction[axis]);
    let mut cell = std::array::from_fn::<_, 3, _>(|axis| {
        let point = point[axis].clamp(0.0, f64::from(dimensions[axis]));
        let floor = point.floor();
        if direction[axis] < 0.0 && point == floor {
            floor as i32 - 1
        } else {
            floor as i32
        }
    });
    let step = direction.map(|axis| {
        if axis > 0.0 {
            1
        } else if axis < 0.0 {
            -1
        } else {
            0
        }
    });
    if normal == [0; 3] {
        let mut axis = 0;
        for candidate in 1..3 {
            if direction[candidate].abs() > direction[axis].abs() {
                axis = candidate;
            }
        }
        normal[axis] = -step[axis];
    }
    let delta = direction.map(|axis| {
        if axis == 0.0 {
            f64::INFINITY
        } else {
            axis.abs().recip()
        }
    });
    let mut next = std::array::from_fn::<_, 3, _>(|axis| {
        if step[axis] == 0 {
            return f64::INFINITY;
        }
        let boundary = cell[axis] + i32::from(step[axis] > 0);
        (f64::from(boundary) - origin[axis]) / direction[axis]
    });
    let mut distance = enter;
    // A straight ray crosses each grid plane at most once.
    let budget = dimensions
        .into_iter()
        .map(|size| size as usize)
        .sum::<usize>()
        + 1;
    for _ in 0..budget {
        if (0..3).any(|axis| cell[axis] < 0 || cell[axis] >= dimensions[axis] as i32)
            || distance >= exit
        {
            return Ok(None);
        }
        let position = cell.map(|axis| axis as u32);
        let material = volume.cell(position);
        if material != EMPTY {
            let surface = volume.material(material).expect("validated voxel palette");
            if surface.geometry() == crate::MaterialGeometry::Cross {
                if let Some(hit) = foliage::intersect_cross(
                    foliage::CellRay {
                        origin,
                        direction,
                        cell: position,
                        entry: distance,
                        exit: next.into_iter().fold(exit, f64::min),
                    },
                    surface,
                    material,
                ) {
                    return Ok(Some(hit));
                }
            } else {
                let point = std::array::from_fn(|axis| {
                    origin[axis] + (distance + SURFACE_OFFSET) * direction[axis]
                });
                let (face, uv) = face_uv(normal, point);
                let color = volume
                    .material(material)
                    .expect("validated voxel palette")
                    .sample(face, uv);
                if color != EMPTY {
                    return Ok(Some(Hit {
                        position,
                        normal: normal.map(f64::from),
                        distance,
                        material,
                        color,
                    }));
                }
            }
        }
        distance = next.into_iter().fold(f64::INFINITY, f64::min);
        if distance >= exit {
            return Ok(None);
        }
        normal = [0; 3];
        // Advance every tied axis, so corner-only contacts do not become phantom voxels.
        for axis in 0..3 {
            if next[axis] == distance {
                cell[axis] += step[axis];
                next[axis] += delta[axis];
                if normal == [0; 3] {
                    normal[axis] = -step[axis];
                }
            }
        }
    }
    Err("voxel ray exceeded its grid-plane traversal bound".into())
}
