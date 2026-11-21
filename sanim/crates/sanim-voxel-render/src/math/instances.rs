use sanim_voxel::{CHUNK_EDGE, MaterialId};

/// Prove that an entire world brick is covered by one uniform source material.
/// An unaligned placement can intersect at most eight source bricks. This avoids
/// flattening 512 cells and preserves the canonical empty-brick GPU fast path.
pub(crate) fn uniform_brick(
    origin: [u32; 3],
    dimensions: [u32; 3],
    brick: [u32; 3],
    page: impl Fn([u32; 3]) -> Option<MaterialId>,
) -> Option<MaterialId> {
    let mut first = [0; 3];
    let mut last = [0; 3];
    for axis in 0..3 {
        let min = brick[axis] * CHUNK_EDGE;
        let max = min + CHUNK_EDGE - 1;
        if min < origin[axis] || max - origin[axis] >= dimensions[axis] {
            return None;
        }
        first[axis] = (min - origin[axis]) / CHUNK_EDGE;
        last[axis] = (max - origin[axis]) / CHUNK_EDGE;
    }
    let mut material = None;
    for z in first[2]..=last[2] {
        for y in first[1]..=last[1] {
            for x in first[0]..=last[0] {
                let next = page([x, y, z])?;
                if material.is_some_and(|previous| previous != next) {
                    return None;
                }
                material = Some(next);
            }
        }
    }
    material
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unaligned_uniform_promotion_requires_every_touched_source_page_and_full_coverage() {
        assert_eq!(uniform_brick([1; 3], [16; 3], [1; 3], |_| Some(7)), Some(7));
        assert_eq!(uniform_brick([1; 3], [16; 3], [0; 3], |_| Some(7)), None);
        assert_eq!(uniform_brick([1; 3], [8; 3], [1; 3], |_| Some(7)), None);
        assert_eq!(
            uniform_brick([1; 3], [16; 3], [1; 3], |p| Some(if p == [1; 3] {
                8
            } else {
                7
            })),
            None
        );
        assert_eq!(
            uniform_brick([1; 3], [16; 3], [1; 3], |p| if p == [1; 3] {
                None
            } else {
                Some(7)
            }),
            None
        );
    }
}
