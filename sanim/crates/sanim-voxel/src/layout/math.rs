//! Exact measurement and placement for the three-dimensional layout tree.
use super::{
    Content, Coordinate, Dimension, Justify, MAX_DEPTH, MAX_LAYOUT_WORK, MAX_NODES, Node, Placed,
    Placement, Plan, Resolved,
};
use crate::Volume;
use fraction::Fraction;
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, ToPrimitive, Zero};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) fn percent(value: Fraction) -> Result<(), String> {
    if !value.is_finite() || value < Fraction::zero() {
        return Err("voxel layout percentages must be finite and nonnegative".into());
    }
    Ok(())
}
fn relative(value: Fraction, extent: u32) -> Result<Fraction, String> {
    percent(value)?;
    value
        .checked_div(&Fraction::from(100))
        .and_then(|value| value.checked_mul(&Fraction::from(extent)))
        .filter(Fraction::is_finite)
        .ok_or_else(|| "voxel layout percentage exceeds exact arithmetic limits".into())
}
pub(super) fn validate_placement(placement: &Placement) -> Result<(), String> {
    for coordinate in placement.at.into_iter().chain(placement.anchor) {
        match coordinate {
            Coordinate::Percent(value) => percent(value)?,
            Coordinate::Cells(value) if !value.is_finite() => {
                return Err("voxel cell coordinates must be finite".into());
            }
            Coordinate::Cells(_) => {}
        }
    }
    if placement.offset.iter().any(|value| !value.is_finite()) {
        return Err("voxel layout offsets must be finite".into());
    }
    if !placement.absolute
        && (placement.at != [Coordinate::default(); 3]
            || placement.anchor != [Coordinate::default(); 3])
    {
        return Err("voxel at/anchor placement requires position_mode=\"absolute\"; relative children use their parent's flow and alignment".into());
    }
    Ok(())
}

struct Measured<'a> {
    node: &'a Node,
    size: [u32; 3],
    children: Vec<Measured<'a>>,
}

fn measure<'a>(
    node: &'a Node,
    available: [Option<u32>; 3],
    depth: usize,
    count: &mut usize,
) -> Result<Measured<'a>, String> {
    if depth > MAX_DEPTH || *count >= MAX_NODES {
        return Err(format!(
            "voxel layout exceeds its {MAX_DEPTH}-level or {MAX_NODES}-node budget"
        ));
    }
    *count += 1;
    let Content::Space(space) = &node.content else {
        let Content::Part { volume, .. } = &node.content else {
            unreachable!()
        };
        return Ok(Measured {
            node,
            size: volume.dimensions(),
            children: Vec::new(),
        });
    };
    let mut definite = [None; 3];
    for axis in 0..3 {
        definite[axis] = match space.size[axis] {
            Dimension::Cells(value) => Some(value),
            Dimension::Percent(value) => {
                percent(value)?;
                let parent = available[axis].ok_or_else(|| format!("voxel layout axis {} has a parent-relative size inside an automatic or missing parent size; give that parent axis a definite size", ["X", "Y", "Z"][axis]))?;
                Some(
                    relative(value, parent)?
                        .floor()
                        .to_u32()
                        .ok_or("voxel percentage size exceeds the cell range")?,
                )
            }
            Dimension::Auto => None,
        };
    }
    let mut inner = [None; 3];
    for axis in 0..3 {
        let padding = space.padding[axis]
            .checked_mul(2)
            .ok_or("voxel padding overflow")?;
        inner[axis] = definite[axis]
            .map(|size| {
                size.checked_sub(padding)
                    .ok_or("voxel padding exceeds the containing size")
            })
            .transpose()?;
    }
    let children = space
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            measure(child, inner, depth + 1, count)
                .map_err(|error| format!("child[{index}]: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let flowing: Vec<_> = children
        .iter()
        .filter(|child| !child.node.placement().absolute)
        .collect();
    let main = space.flow.index();
    let mut natural = [0u32; 3];
    for child in &flowing {
        for (axis, value) in natural.iter_mut().enumerate() {
            *value = if axis == main {
                value
                    .checked_add(child.size[axis])
                    .ok_or("voxel layout size overflow")?
            } else {
                (*value).max(child.size[axis])
            };
        }
    }
    natural[main] = natural[main]
        .checked_add(
            space
                .gap
                .checked_mul(flowing.len().saturating_sub(1) as u32)
                .ok_or("voxel gap overflow")?,
        )
        .ok_or("voxel layout size overflow")?;
    let mut size = [0; 3];
    for axis in 0..3 {
        let required = natural[axis]
            .checked_add(
                space.padding[axis]
                    .checked_mul(2)
                    .ok_or("voxel padding overflow")?,
            )
            .ok_or("voxel layout size overflow")?;
        size[axis] = definite[axis].unwrap_or(required);
        if size[axis] == 0 {
            return Err(format!(
                "voxel layout axis {} resolves to zero cells; give an empty or absolute-only container a definite size",
                ["X", "Y", "Z"][axis]
            ));
        }
        if size[axis] < required {
            return Err(format!(
                "voxel layout axis {} overflows: children and padding need {required} cells, containing size is {}",
                ["X", "Y", "Z"][axis],
                size[axis]
            ));
        }
    }
    Ok(Measured {
        node,
        size,
        children,
    })
}

pub fn coordinate(value: Coordinate, extent: u32) -> Result<Fraction, String> {
    match value {
        Coordinate::Cells(value) => Ok(value),
        Coordinate::Percent(value) => value
            .checked_div(&Fraction::from(100))
            .and_then(|value| value.checked_mul(&Fraction::from(extent)))
            .filter(Fraction::is_finite)
            .ok_or_else(|| "relative coordinate exceeds exact arithmetic limits".into()),
    }
}
fn distributed(justify: Justify, extra: u32, index: usize, count: usize) -> Fraction {
    let extra = u64::from(extra);
    let index = index as u64;
    let count = count as u64;
    match justify {
        Justify::Start => Fraction::from(0),
        Justify::Center => Fraction::new(extra, 2u64),
        Justify::End => Fraction::from(extra),
        Justify::Between if count > 1 => Fraction::new(extra * index, count - 1),
        Justify::Between => Fraction::from(0),
        Justify::Around => Fraction::new(extra * (2 * index + 1), 2 * count),
        Justify::Evenly => Fraction::new(extra * (index + 1), count + 1),
    }
}

fn arrange(
    measured: &Measured<'_>,
    origin: [u32; 3],
    path: &mut Vec<usize>,
    placed: &mut Vec<Placed>,
    parts: &mut Vec<(Arc<Volume>, [i32; 3])>,
) -> Result<(), String> {
    let leaf = matches!(measured.node.content, Content::Part { .. });
    placed.push(Placed {
        path: path.clone(),
        origin,
        size: measured.size,
        leaf,
    });
    let Content::Space(space) = &measured.node.content else {
        let Content::Part { volume: part, .. } = &measured.node.content else {
            unreachable!()
        };
        parts.push((part.clone(), origin.map(|value| value as i32)));
        return Ok(());
    };
    let inner =
        std::array::from_fn::<_, 3, _>(|axis| measured.size[axis] - 2 * space.padding[axis]);
    let main = space.flow.index();
    let flowing = measured
        .children
        .iter()
        .filter(|child| !child.node.placement().absolute)
        .count();
    let used = measured
        .children
        .iter()
        .filter(|child| !child.node.placement().absolute)
        .map(|child| child.size[main])
        .sum::<u32>()
        + space.gap * flowing.saturating_sub(1) as u32;
    let extra = inner[main] - used;
    let mut cursor = 0u32;
    let mut flow_index = 0;
    for (index, child) in measured.children.iter().enumerate() {
        let placement = child.node.placement();
        let mut offset = [0i32; 3];
        for axis in 0..3 {
            if placement.absolute {
                offset[axis] = coordinate(placement.at[axis], inner[axis])?
                    .checked_sub(&coordinate(placement.anchor[axis], child.size[axis])?)
                    .and_then(|value| value.checked_add(&placement.offset[axis]))
                    .ok_or("voxel absolute placement exceeds exact arithmetic limits")?
                    .floor()
                    .to_i32()
                    .ok_or("voxel absolute placement exceeds the cell range")?;
            } else {
                let value = if axis == main {
                    Fraction::from(cursor) + distributed(space.justify, extra, flow_index, flowing)
                } else {
                    let free = inner[axis].checked_sub(child.size[axis]).ok_or_else(|| {
                        format!(
                            "child[{index}] exceeds its parent's {} axis",
                            ["X", "Y", "Z"][axis]
                        )
                    })?;
                    placement.align.unwrap_or(space.align)[axis].shift(free)
                };
                offset[axis] = value
                    .checked_add(&placement.offset[axis])
                    .ok_or("voxel offset exceeds exact arithmetic limits")?
                    .floor()
                    .to_i32()
                    .ok_or("voxel layout position exceeds the cell range")?;
            }
            if offset[axis] < 0
                || (offset[axis] as u64 + u64::from(child.size[axis])) > u64::from(inner[axis])
            {
                return Err(format!(
                    "child[{index}] overflows parent axis {}: origin {}, size {}, available {}; adjust its anchor, offset, or containing size",
                    ["X", "Y", "Z"][axis],
                    offset[axis],
                    child.size[axis],
                    inner[axis]
                ));
            }
        }
        if !placement.absolute {
            cursor += child.size[main];
            flow_index += 1;
            if flow_index < flowing {
                cursor += space.gap;
            }
        }
        let position =
            std::array::from_fn(|axis| origin[axis] + space.padding[axis] + offset[axis] as u32);
        path.push(index);
        arrange(child, position, path, placed, parts)
            .map_err(|error| format!("child[{index}]: {error}"))?;
        path.pop();
    }
    Ok(())
}

/// Measure first, then place. No camera, renderer, or external pixel dimensions participate.
pub fn prepare(node: &Arc<Node>) -> Result<Plan, String> {
    let mut work = 0;
    let measured = measure(node, [None; 3], 0, &mut work)?;
    crate::math::validate_dimensions(measured.size, crate::MAX_DIMENSION)?;
    let mut placements = Vec::new();
    let mut parts = Vec::new();
    arrange(
        &measured,
        [0; 3],
        &mut Vec::new(),
        &mut placements,
        &mut parts,
    )?;
    // Preparing instance metadata never copies voxels. Explicit flattening meters its own copy work.
    for (part, _) in &parts {
        work = work
            .checked_add(part.chunk_count().saturating_mul(8))
            .ok_or("voxel layout work count overflow")?;
        if work > MAX_LAYOUT_WORK {
            return Err(format!(
                "voxel layout exceeds its {MAX_LAYOUT_WORK}-operation metadata budget; reduce repeated occupied bricks"
            ));
        }
    }
    let leaves: Vec<_> = parts
        .iter()
        .map(|(part, origin)| (part.revision(), origin))
        .collect();
    let encoded =
        serde_json::to_vec(&("snapped-voxel-assembly", measured.size, &placements, leaves))
            .map_err(|error| error.to_string())?;
    Ok(Plan {
        dimensions: measured.size,
        placements: placements.into(),
        work,
        parts,
        fingerprint: Sha256::digest(encoded).into(),
    })
}

impl Plan {
    pub fn assemble_instances(self) -> Result<Resolved, String> {
        let parts = self
            .parts
            .into_iter()
            .map(|(asset, origin)| {
                let mut cells = [0; 3];
                for axis in 0..3 {
                    cells[axis] = u32::try_from(origin[axis])
                        .map_err(|_| "resolved instance origin is negative")?;
                }
                Ok((asset, cells))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let scene = crate::InstanceScene::new(self.dimensions, parts)?;
        Ok(Resolved {
            source: crate::SceneSource::Instances(Arc::new(scene)),
            placements: self.placements,
            work: self.work,
        })
    }
    pub fn assemble(self) -> Result<Resolved, String> {
        let volume = crate::math::instances::flatten(
            self.dimensions,
            self.parts
                .iter()
                .map(|(part, origin)| (part.as_ref(), *origin)),
        )?;
        Ok(Resolved {
            source: crate::SceneSource::Volume(Arc::new(volume)),
            placements: self.placements,
            work: self.work,
        })
    }
}

/// Measure, validate, and assemble with one canonical implementation.
pub fn resolve(node: &Arc<Node>) -> Result<Resolved, String> {
    prepare(node)?.assemble()
}
