//! Immutable instanced occupancy: original assets, canonical palette, and bounded spatial candidates.
use crate::{Material, MaterialId, MaterialPalette, Volume, revision};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
pub const MAX_INSTANCES: usize = 4096;
pub const MAX_BUCKET_CANDIDATES: usize = 64;
pub const MAX_BUCKET_ENTRIES: usize = 2 * 1024 * 1024;
pub const MAX_BUCKET_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Instance {
    pub volume: Arc<Volume>,
    pub origin: [u32; 3],
    pub materials: Arc<[MaterialId]>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bucket {
    pub coordinate: [u32; 3],
    pub start: u32,
    pub len: u32,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BucketIndex {
    pub buckets: Vec<Bucket>,
    pub indices: Vec<u32>,
}
impl BucketIndex {
    pub fn candidates(&self, coordinate: [u32; 3]) -> &[u32] {
        let Ok(index) = self
            .buckets
            .binary_search_by_key(&coordinate, |bucket| bucket.coordinate)
        else {
            return &[];
        };
        let bucket = self.buckets[index];
        &self.indices[bucket.start as usize..(bucket.start + bucket.len) as usize]
    }
    pub fn resident_bytes(&self) -> usize {
        self.buckets.len() * std::mem::size_of::<Bucket>()
            + self.indices.len() * std::mem::size_of::<u32>()
    }
}
#[derive(Clone, Debug)]
pub struct InstanceScene {
    dimensions: [u32; 3],
    instances: Arc<[Instance]>,
    palette: Arc<MaterialPalette>,
    buckets: Arc<BucketIndex>,
    revision: u64,
    resident_bytes: usize,
    metadata_bytes: usize,
}
impl PartialEq for InstanceScene {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            || self.dimensions == other.dimensions
                && self.instances.len() == other.instances.len()
                && self
                    .instances
                    .iter()
                    .zip(other.instances.iter())
                    .all(|(a, b)| {
                        a.origin == b.origin && a.volume == b.volume && a.materials == b.materials
                    })
    }
}
impl Eq for InstanceScene {}
impl InstanceScene {
    pub fn new(dimensions: [u32; 3], parts: Vec<(Arc<Volume>, [u32; 3])>) -> Result<Self, String> {
        crate::math::validate_dimensions(dimensions, crate::MAX_DIMENSION)?;
        if parts.len() > MAX_INSTANCES {
            return Err(format!(
                "instance scene has {} parts; maximum {MAX_INSTANCES}",
                parts.len()
            ));
        }
        let mut palette = Arc::new(MaterialPalette::default());
        let mut assets = BTreeMap::new();
        let mut storages = BTreeSet::new();
        let mut palettes = BTreeSet::new();
        let mut instances = Vec::with_capacity(parts.len());
        let mut metadata_bytes = std::mem::size_of::<Self>()
            + std::mem::size_of::<BucketIndex>()
            + 8 * std::mem::size_of::<usize>()
            + parts.len() * std::mem::size_of::<Instance>();
        let mut bytes = metadata_bytes;
        for (index, (volume, origin)) in parts.into_iter().enumerate() {
            for axis in 0..3 {
                if origin[axis]
                    .checked_add(volume.dimensions()[axis])
                    .is_none_or(|end| end > dimensions[axis])
                {
                    return Err(format!(
                        "instance[{index}] origin {origin:?} plus asset size {:?} exceeds containing dimensions {dimensions:?} on axis {axis}",
                        volume.dimensions()
                    ));
                }
            }
            let materials = if let Some(materials) = assets.get(&volume.revision()) {
                Arc::clone(materials)
            } else {
                let (next, materials) = palette.with_volume(&volume)?;
                palette = next;
                bytes = bytes
                    .saturating_add(
                        std::mem::size_of::<Volume>() + 2 * std::mem::size_of::<usize>(),
                    )
                    .saturating_add(materials.len() * std::mem::size_of::<MaterialId>());
                if storages.insert(volume.storage_identity()) {
                    bytes = bytes
                        .saturating_add(volume.resident_bytes() - volume.palette().resident_bytes())
                        .saturating_add(volume.storage_metadata_bytes_since(None));
                }
                if palettes.insert(volume.palette_identity()) {
                    bytes = bytes
                        .saturating_add(volume.palette().resident_bytes())
                        .saturating_add(
                            std::mem::size_of::<MaterialPalette>()
                                + 2 * std::mem::size_of::<usize>(),
                        );
                }
                metadata_bytes = metadata_bytes.saturating_add(
                    materials.len() * std::mem::size_of::<MaterialId>()
                        + 2 * std::mem::size_of::<usize>(),
                );
                bytes = bytes.saturating_add(2 * std::mem::size_of::<usize>());
                assets.insert(volume.revision(), materials.clone());
                materials
            };
            if bytes > crate::MAX_RESIDENT_BYTES {
                return Err(format!(
                    "instance assets and metadata exceed the {} byte resident budget; reuse assets or reduce their occupied chunks",
                    crate::MAX_RESIDENT_BYTES
                ));
            }
            instances.push(Instance {
                volume,
                origin,
                materials,
            });
        }
        let buckets = Arc::new(crate::math::instances::buckets(&instances)?);
        bytes = bytes
            .saturating_add(buckets.resident_bytes())
            .saturating_add(palette.metadata_bytes());
        if bytes > crate::MAX_RESIDENT_BYTES {
            return Err(
                "instance scene palette, assets, and spatial metadata exceed the resident budget"
                    .into(),
            );
        }
        metadata_bytes = metadata_bytes
            .saturating_add(buckets.resident_bytes())
            .saturating_add(palette.metadata_bytes());
        Ok(Self {
            dimensions,
            instances: instances.into(),
            palette,
            buckets,
            revision: revision(),
            resident_bytes: bytes,
            metadata_bytes,
        })
    }
    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }
    pub fn palette(&self) -> &Arc<MaterialPalette> {
        &self.palette
    }
    pub fn palette_revision(&self) -> u64 {
        self.palette.revision()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn buckets(&self) -> &BucketIndex {
        &self.buckets
    }
    pub fn resident_bytes(&self) -> usize {
        self.resident_bytes
    }
    /// Structural storage only; asset payloads are charged separately by their existing weak resource proofs.
    pub fn metadata_bytes(&self) -> usize {
        self.metadata_bytes
    }
    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Arc<Material>)> {
        self.palette.materials()
    }
    pub fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        self.palette.material(id)
    }
    pub fn occupied_voxels_upper_bound(&self) -> usize {
        self.instances
            .iter()
            .map(|instance| instance.volume.occupied_voxels())
            .fold(0usize, usize::saturating_add)
            .min(
                self.dimensions
                    .map(|size| size as usize)
                    .into_iter()
                    .product(),
            )
    }
    /// Explicit materialization for callers that require a full voxel grid. Rendering never calls this.
    pub fn flatten(&self) -> Result<Arc<Volume>, String> {
        let mut volume = crate::math::instances::flatten(
            self.dimensions,
            self.instances.iter().map(|instance| {
                (
                    instance.volume.as_ref(),
                    instance.origin.map(|axis| axis as i32),
                )
            }),
        )?;
        volume.palette = self.palette.clone();
        Ok(Arc::new(volume))
    }
    pub fn cell(&self, position: [u32; 3]) -> MaterialId {
        for &id in self
            .buckets
            .candidates(position.map(|axis| axis / crate::CHUNK_EDGE))
            .iter()
            .rev()
        {
            let instance = &self.instances[id as usize];
            let mut local = [0; 3];
            let mut inside = true;
            for axis in 0..3 {
                if position[axis] < instance.origin[axis]
                    || position[axis] - instance.origin[axis] >= instance.volume.dimensions()[axis]
                {
                    inside = false;
                    break;
                }
                local[axis] = position[axis] - instance.origin[axis];
            }
            if inside {
                let material = instance.volume.cell(local);
                if material != crate::EMPTY {
                    return instance.materials[material as usize];
                }
            }
        }
        crate::EMPTY
    }
}

#[derive(Serialize, Deserialize)]
struct EncodedPart {
    asset: usize,
    origin: [u32; 3],
}
#[derive(Serialize, Deserialize)]
struct EncodedScene {
    dimensions: [u32; 3],
    #[serde(deserialize_with = "decode_assets")]
    assets: Vec<Arc<Volume>>,
    #[serde(deserialize_with = "crate::bounded::<_,_,MAX_INSTANCES>")]
    instances: Vec<EncodedPart>,
}
fn decode_assets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Arc<Volume>>, D::Error> {
    struct Assets;
    impl<'de> serde::de::Visitor<'de> for Assets {
        type Value = Vec<Arc<Volume>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "at most {MAX_INSTANCES} assets within the resident byte budget"
            )
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            if sequence
                .size_hint()
                .is_some_and(|count| count > MAX_INSTANCES)
            {
                return Err(serde::de::Error::custom(
                    "instance asset count exceeds its budget",
                ));
            }
            let mut assets = Vec::new();
            let mut bytes = 0usize;
            while let Some(asset) = sequence.next_element::<Arc<Volume>>()? {
                bytes = bytes
                    .saturating_add(asset.resident_bytes())
                    .saturating_add(asset.storage_metadata_bytes_since(None));
                if assets.len() == MAX_INSTANCES || bytes > crate::MAX_RESIDENT_BYTES {
                    return Err(serde::de::Error::custom(
                        "instance assets exceed the count or resident byte budget",
                    ));
                }
                assets.push(asset);
            }
            Ok(assets)
        }
    }
    deserializer.deserialize_seq(Assets)
}

impl Serialize for InstanceScene {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut identities = BTreeMap::new();
        let mut assets = Vec::new();
        let mut instances = Vec::with_capacity(self.instances.len());
        for instance in self.instances.iter() {
            let asset = *identities
                .entry(instance.volume.revision())
                .or_insert_with(|| {
                    let id = assets.len();
                    assets.push(instance.volume.clone());
                    id
                });
            instances.push(EncodedPart {
                asset,
                origin: instance.origin,
            });
        }
        EncodedScene {
            dimensions: self.dimensions,
            assets,
            instances,
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for InstanceScene {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = EncodedScene::deserialize(deserializer)?;
        let parts = encoded
            .instances
            .into_iter()
            .map(|instance| {
                encoded
                    .assets
                    .get(instance.asset)
                    .cloned()
                    .map(|asset| (asset, instance.origin))
                    .ok_or_else(|| {
                        serde::de::Error::custom(format!(
                            "instance asset index {} is missing",
                            instance.asset
                        ))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(encoded.dimensions, parts).map_err(serde::de::Error::custom)
    }
}
