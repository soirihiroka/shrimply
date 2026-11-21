//! Sparse occupied-cell tests and camera projection checks share canonical layout/camera data.
use super::{Budget, Statistics};
use crate::{
    CHUNK_EDGE, Camera, EMPTY, Volume,
    layout::{Node, Placed},
};
use glam::DVec3;

struct Leaf<'a> {
    volume: &'a Volume,
    placed: &'a Placed,
}
struct Occurrence<'a> {
    placed: &'a Placed,
    index: usize,
}

fn node_at<'a>(root: &'a Node, path: &[usize], budget: &mut Budget) -> Result<&'a Node, String> {
    let mut node = root;
    for &index in path {
        budget.charge()?;
        node = node
            .children()
            .and_then(|children| children.get(index))
            .ok_or("voxel assertion received layout metadata for a different tree")?;
    }
    Ok(node)
}

fn occurrences<'a>(
    root: &'a Node,
    placements: &'a [Placed],
    target: &Node,
    budget: &mut Budget,
) -> Result<Vec<Occurrence<'a>>, String> {
    let mut result = Vec::new();
    for (index, placed) in placements.iter().enumerate() {
        budget.charge()?;
        let node = node_at(root, &placed.path, budget)?;
        if !node.same_recipe(target) {
            continue;
        }
        result.push(Occurrence { placed, index });
    }
    if result.is_empty() {
        return Err("voxel assertion target is not part of this layout; select a VoxelPart or VoxelBox recipe included in the containing layout; all identical recipe occurrences are checked".into());
    }
    Ok(result)
}

// Canonical layout metadata is preorder, so each subtree occupies one contiguous range.
fn leaves<'a>(
    root: &'a Node,
    placements: &'a [Placed],
    occurrence: &Occurrence<'a>,
    budget: &mut Budget,
) -> Result<Vec<Leaf<'a>>, String> {
    let mut result = Vec::new();
    for placed in &placements[occurrence.index..] {
        budget.charge()?;
        if !placed.path.starts_with(&occurrence.placed.path) {
            break;
        }
        if !placed.leaf {
            continue;
        }
        let volume = node_at(root, &placed.path, budget)?
            .volume()
            .ok_or("voxel assertion leaf has no voxel grid")?;
        if placed.size != volume.dimensions() {
            return Err("voxel assertion leaf dimensions do not match its resolved grid".into());
        }
        result.push(Leaf { volume, placed });
    }
    Ok(result)
}

fn upper(placed: &Placed) -> Result<[u32; 3], String> {
    let mut result = [0; 3];
    for (axis, value) in result.iter_mut().enumerate() {
        *value = placed.origin[axis]
            .checked_add(placed.size[axis])
            .ok_or("voxel assertion bounds overflow")?;
    }
    Ok(result)
}

fn overlap(
    first: &Leaf<'_>,
    second: &Leaf<'_>,
    budget: &mut Budget,
) -> Result<Option<[u32; 3]>, String> {
    budget.charge()?;
    budget.statistics.pairs += 1;
    let first_max = upper(first.placed)?;
    let second_max = upper(second.placed)?;
    let minimum = std::array::from_fn::<_, 3, _>(|axis| {
        first.placed.origin[axis].max(second.placed.origin[axis])
    });
    let maximum = std::array::from_fn::<_, 3, _>(|axis| first_max[axis].min(second_max[axis]));
    if (0..3).any(|axis| minimum[axis] >= maximum[axis]) {
        return Ok(None);
    }
    let (small, other) = if first.volume.chunk_count() <= second.volume.chunk_count() {
        (first, second)
    } else {
        (second, first)
    };
    for (chunk_position, chunk) in small.volume.chunks() {
        budget.charge()?;
        let start = std::array::from_fn::<_, 3, _>(|axis| {
            small.placed.origin[axis] + chunk_position[axis] * CHUNK_EDGE
        });
        let low = std::array::from_fn::<_, 3, _>(|axis| minimum[axis].max(start[axis]));
        let high = std::array::from_fn::<_, 3, _>(|axis| {
            maximum[axis].min(start[axis].saturating_add(CHUNK_EDGE))
        });
        if (0..3).any(|axis| low[axis] >= high[axis]) {
            continue;
        }
        for z in low[2]..high[2] {
            for y in low[1]..high[1] {
                for x in low[0]..high[0] {
                    budget.charge()?;
                    budget.statistics.tested_cells += 1;
                    let world = [x, y, z];
                    if chunk.get(std::array::from_fn(|axis| world[axis] - start[axis])) != EMPTY
                        && other.volume.get(std::array::from_fn(|axis| {
                            world[axis] - other.placed.origin[axis]
                        }))? != EMPTY
                    {
                        return Ok(Some(world));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn path(path: &[usize]) -> String {
    if path.is_empty() {
        return "root".into();
    }
    path.iter()
        .map(|index| format!("child[{index}]"))
        .collect::<Vec<_>>()
        .join(".")
}

/// Verify every matching recipe occurrence. Two equal recipes check distinct instances.
/// Recipe matching is semantic, making assertions safe under pure-function memoization.
pub fn assert_disjoint(
    root: &Node,
    placements: &[Placed],
    first: &Node,
    second: &Node,
    maximum_work: usize,
) -> Result<Statistics, String> {
    let mut budget = Budget::new(maximum_work);
    let first_occurrences = occurrences(root, placements, first, &mut budget)?;
    let second_occurrences = occurrences(root, placements, second, &mut budget)?;
    let same = first.same_recipe(second);
    if same && first_occurrences.len() < 2 {
        return Err("assert_disjoint requires two selected parts, or a reused part with at least two distinct occurrences".into());
    }
    let first_leaves = first_occurrences
        .iter()
        .map(|occurrence| leaves(root, placements, occurrence, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    let second_leaves = second_occurrences
        .iter()
        .map(|occurrence| leaves(root, placements, occurrence, &mut budget))
        .collect::<Result<Vec<_>, _>>()?;
    for (first_index, first_occurrence) in first_leaves.iter().enumerate() {
        for (second_index, second_occurrence) in second_leaves.iter().enumerate() {
            if same && second_index <= first_index {
                continue;
            }
            budget.charge()?;
            for first_leaf in first_occurrence {
                for second_leaf in second_occurrence {
                    if let Some(cell) = overlap(first_leaf, second_leaf, &mut budget)? {
                        return Err(format!(
                            "V001: assert_disjoint failed: {} and {} occupy the same world voxel {cell:?}; move one part, adjust its size, or remove the conflicting cells",
                            path(&first_leaf.placed.path),
                            path(&second_leaf.placed.path)
                        ));
                    }
                }
            }
        }
    }
    Ok(budget.statistics)
}

/// Check allocated bounds, rather than claiming mesh/voxel occupancy containment.
pub fn assert_within(
    root: &Node,
    placements: &[Placed],
    inner: &Node,
    outer: &Node,
    maximum_work: usize,
) -> Result<Statistics, String> {
    let mut budget = Budget::new(maximum_work);
    let inners = occurrences(root, placements, inner, &mut budget)?;
    let outers = occurrences(root, placements, outer, &mut budget)?;
    for inner in inners {
        let inner_max = upper(inner.placed)?;
        let mut contained = false;
        for outer in &outers {
            budget.charge()?;
            budget.statistics.pairs += 1;
            let outer_max = upper(outer.placed)?;
            if (0..3).all(|axis| {
                inner.placed.origin[axis] >= outer.placed.origin[axis]
                    && inner_max[axis] <= outer_max[axis]
            }) {
                contained = true;
                break;
            }
        }
        if !contained {
            return Err(format!(
                "V002: assert_within failed: {} has world bounds {:?}..{inner_max:?}, outside selected container {} ({} matching occurrences); adjust its anchor, offset, or the containing size",
                path(&inner.placed.path),
                inner.placed.origin,
                path(&outers[0].placed.path),
                outers.len()
            ));
        }
    }
    Ok(budget.statistics)
}

/// Check all eight declared-volume corners at this camera snapshot. Margin is a fraction of each image edge.
pub fn assert_framed(
    camera: &Camera,
    dimensions: [u32; 3],
    viewport: [u32; 2],
    margin: f64,
) -> Result<Statistics, String> {
    if !margin.is_finite() || !(0.0..0.5).contains(&margin) {
        return Err("voxel framing margin must be finite and between 0% and 50%".into());
    }
    if dimensions.contains(&0) {
        return Err("voxel framing dimensions must be nonzero".into());
    }
    let basis = camera.basis(viewport)?;
    let position = DVec3::from_array(basis.position.map(f64::from));
    let forward = DVec3::from_array(basis.forward.map(f64::from));
    let right = DVec3::from_array(basis.right.map(f64::from));
    let up = DVec3::from_array(basis.up.map(f64::from));
    let limit = 1.0 - 2.0 * margin;
    for corner in 0..8 {
        let point = std::array::from_fn::<_, 3, _>(|axis| {
            if corner & (1 << axis) == 0 {
                0.0
            } else {
                f64::from(dimensions[axis])
            }
        });
        let delta = DVec3::from_array(point) - position;
        let depth = delta.dot(forward);
        let distance = if basis.perspective { depth } else { 1.0 };
        let x =
            delta.dot(right) / (f64::from(basis.half_height) * f64::from(basis.aspect) * distance);
        let y = delta.dot(up) / (f64::from(basis.half_height) * distance);
        if depth <= 0.0 || !x.is_finite() || !y.is_finite() || x.abs() > limit || y.abs() > limit {
            return Err(format!(
                "V003: assert_framed failed: declared-volume corner {point:?} projects outside the image margin (normalized image position [{x:.4}, {y:.4}], depth {depth:.4}); lower zoom, widen field of view, or reposition the camera"
            ));
        }
    }
    Ok(Statistics {
        work: 8,
        pairs: 0,
        tested_cells: 0,
    })
}
