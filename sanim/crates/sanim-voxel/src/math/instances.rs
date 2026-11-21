//! Bucket only occupied asset bricks; integer translation touches at most eight world bricks.
use crate::{
    Bucket, BucketIndex, CHUNK_EDGE, Instance, MAX_BUCKET_BYTES, MAX_BUCKET_CANDIDATES,
    MAX_BUCKET_ENTRIES,
};
use std::collections::BTreeMap;

pub(crate) fn buckets(instances: &[Instance]) -> Result<BucketIndex, String> {
    let mut buckets: BTreeMap<[u32; 3], Vec<u32>> = BTreeMap::new();
    let mut entries = 0;
    for (id, instance) in instances.iter().enumerate() {
        for (chunk, _) in instance.volume.chunks() {
            let start = std::array::from_fn::<_, 3, _>(|axis| {
                instance.origin[axis] + chunk[axis] * CHUNK_EDGE
            });
            let end = std::array::from_fn::<_, 3, _>(|axis| {
                instance.origin[axis]
                    + ((chunk[axis] + 1) * CHUNK_EDGE).min(instance.volume.dimensions()[axis])
                    - 1
            });
            for z in start[2] / CHUNK_EDGE..=end[2] / CHUNK_EDGE {
                for y in start[1] / CHUNK_EDGE..=end[1] / CHUNK_EDGE {
                    for x in start[0] / CHUNK_EDGE..=end[0] / CHUNK_EDGE {
                        let candidates = buckets.entry([x, y, z]).or_default();
                        if candidates.last() == Some(&(id as u32)) {
                            continue;
                        }
                        if candidates.len() == MAX_BUCKET_CANDIDATES {
                            return Err(format!(
                                "instance spatial bucket [{x},{y},{z}] exceeds {MAX_BUCKET_CANDIDATES} candidates; spread overlapping instances or combine local assets"
                            ));
                        }
                        candidates.push(id as u32);
                        entries += 1;
                        if entries > MAX_BUCKET_ENTRIES {
                            return Err(format!(
                                "instance spatial index exceeds {MAX_BUCKET_ENTRIES} entries; reduce repeated occupied bricks"
                            ));
                        }
                        if crate::math::map_metadata_bytes(buckets.len())
                            .saturating_add(entries * std::mem::size_of::<u32>())
                            > MAX_BUCKET_BYTES
                        {
                            return Err(format!(
                                "instance spatial index exceeds its {MAX_BUCKET_BYTES} byte metadata budget; use fewer occupied world buckets"
                            ));
                        }
                    }
                }
            }
        }
    }
    let mut index = BucketIndex {
        buckets: Vec::with_capacity(buckets.len()),
        indices: Vec::with_capacity(entries),
    };
    for (coordinate, candidates) in buckets {
        index.buckets.push(Bucket {
            coordinate,
            start: index.indices.len() as u32,
            len: candidates.len() as u32,
        });
        index.indices.extend(candidates);
    }
    Ok(index)
}

/// One bounded explicit materialization path; instance rendering and ordinary planning never call it.
pub(crate) fn flatten<'a>(
    dimensions: [u32; 3],
    parts: impl Iterator<Item = (&'a crate::Volume, [i32; 3])> + Clone,
) -> Result<crate::Volume, String> {
    let mut volume = crate::Volume::new(dimensions)?;
    let mut work = 0usize;
    for (asset, origin) in parts.clone() {
        work = work
            .checked_add(volume.placement_work(asset, origin)?)
            .ok_or("voxel flatten work count overflow")?;
        if work > crate::layout::MAX_LAYOUT_WORK {
            return Err(format!(
                "explicit voxel flattening exceeds its {}-operation copy budget; render the instance scene directly instead",
                crate::layout::MAX_LAYOUT_WORK
            ));
        }
    }
    for (asset, origin) in parts {
        volume = volume.place(asset, origin)?;
    }
    Ok(volume)
}
