//! Immutable sparse voxel data, independent of the language and GPU renderer.

pub mod instances;
pub use instances::{
    Bucket, BucketIndex, Instance, InstanceScene, MAX_BUCKET_BYTES, MAX_BUCKET_CANDIDATES,
    MAX_BUCKET_ENTRIES, MAX_INSTANCES,
};
pub mod scene_source;
pub use scene_source::{GridView, SceneSource};
pub mod appearance;
pub mod camera;
pub mod layout;
pub mod lighting;
mod material;
mod palette;
pub mod procedural;
pub use palette::MaterialPalette;
pub mod math;
pub mod verify;

pub use appearance::{AmbientOcclusion, AoMode, ShadowMode, Shadows};
pub use camera::{Camera, CameraRig, Projection, Ray, View};
pub use lighting::{
    Environment, EnvironmentParameters, Light, LightKind, LightParameters, Lighting,
    MAX_LIGHT_INTENSITY, MAX_LIGHTS, MIN_LIGHT_DISTANCE, PointLight, SceneLightParameters,
    SunLight, deserialize_lights, scene_light_parameters,
};
pub use material::{
    Material, MaterialGeometry, MaterialId, OpticalProperties, Texture, texture_bytes,
};

use imbl::{OrdMap, ordmap::DiffItem};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::hash::{Hash, Hasher};
use std::sync::{
    Arc, OnceLock, Weak,
    atomic::{AtomicU64, Ordering},
};

pub const CHUNK_EDGE: u32 = 8;
pub const CHUNK_VOXELS: usize = CHUNK_EDGE as usize * CHUNK_EDGE as usize * CHUNK_EDGE as usize;
pub const EMPTY: u32 = 0;
pub const MAX_DIMENSION: u32 = 1024;
pub const MAX_CHUNKS: usize = 65_536;
pub const MAX_RESIDENT_BYTES: usize = 256 * 1024 * 1024;
const ENTRY_BYTES: usize = 256;
const DENSE_BYTES: usize = CHUNK_VOXELS * size_of::<u32>();

fn fingerprint(value: &impl Hash) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hash);
    hash.finish()
}

/// Conservative limits count resident chunks, not the potentially enormous empty volume.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_dimension: u32,
    pub max_chunks: usize,
    pub max_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dimension: MAX_DIMENSION,
            max_chunks: MAX_CHUNKS,
            max_bytes: MAX_RESIDENT_BYTES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
enum Cells {
    Uniform(u32),
    Dense(Box<[u32; CHUNK_VOXELS]>),
}

/// Cells hold one-based material indices. Zero denotes empty space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Chunk {
    cells: Cells,
    occupied: usize,
    fingerprint: u64,
    max_material: MaterialId,
}

impl Chunk {
    fn solid(material: u32) -> Self {
        let cells = Cells::Uniform(material);
        Self {
            fingerprint: fingerprint(&cells),
            cells,
            occupied: CHUNK_VOXELS,
            max_material: material,
        }
    }
    pub fn uniform(&self) -> Option<u32> {
        match self.cells {
            Cells::Uniform(color) => Some(color),
            Cells::Dense(_) => None,
        }
    }

    pub fn get(&self, local: [u32; 3]) -> u32 {
        assert!(
            local.into_iter().all(|axis| axis < CHUNK_EDGE),
            "voxel chunk coordinate is out of bounds"
        );
        match &self.cells {
            Cells::Uniform(color) => *color,
            Cells::Dense(cells) => cells[index(local)],
        }
    }

    pub fn values(&self) -> [u32; CHUNK_VOXELS] {
        match &self.cells {
            Cells::Uniform(color) => [*color; CHUNK_VOXELS],
            Cells::Dense(cells) => **cells,
        }
    }

    pub fn occupied(&self) -> usize {
        self.occupied
    }

    pub fn resident_bytes(&self) -> usize {
        ENTRY_BYTES
            + size_of::<Self>()
            + match self.cells {
                Cells::Uniform(_) => 0,
                Cells::Dense(_) => DENSE_BYTES,
            }
    }

    fn from_cells(cells: [u32; CHUNK_VOXELS]) -> Option<Self> {
        let (occupied, max_material) = cells.iter().fold((0usize, EMPTY), |(count, max), cell| {
            (count + usize::from(*cell != EMPTY), max.max(*cell))
        });
        if occupied == 0 {
            return None;
        }
        let first = cells[0];
        let cells = if cells.iter().all(|cell| *cell == first) {
            Cells::Uniform(first)
        } else {
            Cells::Dense(Box::new(cells))
        };
        Some(Self {
            fingerprint: fingerprint(&cells),
            cells,
            occupied,
            max_material,
        })
    }
}

impl Serialize for Chunk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.cells {
            Cells::Uniform(material) => material.serialize(serializer),
            Cells::Dense(cells) => cells.as_slice().serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Chunk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Encoded;
        impl<'de> serde::de::Visitor<'de> for Encoded {
            type Value = Chunk;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    formatter,
                    "one material index or exactly {CHUNK_VOXELS} voxel cells"
                )
            }
            fn visit_u64<E: serde::de::Error>(self, material: u64) -> Result<Chunk, E> {
                let material = u32::try_from(material)
                    .map_err(|_| E::custom("voxel material index exceeds u32"))?;
                if material == EMPTY {
                    return Err(E::custom("empty voxel chunks must be omitted"));
                }
                Ok(Chunk::solid(material))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Chunk, A::Error> {
                let mut cells = [EMPTY; CHUNK_VOXELS];
                for cell in &mut cells {
                    *cell = seq.next_element()?.ok_or_else(|| {
                        serde::de::Error::custom("voxel chunk contains too few cells")
                    })?;
                }
                if seq.next_element::<u32>()?.is_some() {
                    return Err(serde::de::Error::custom(
                        "voxel chunk contains too many cells",
                    ));
                }
                Chunk::from_cells(cells)
                    .ok_or_else(|| serde::de::Error::custom("empty voxel chunks must be omitted"))
            }
        }
        deserializer.deserialize_any(Encoded)
    }
}

#[derive(Debug)]
struct Storage {
    chunks: OrdMap<[u32; 3], SharedChunk>,
    bytes: usize,
    occupied: usize,
    parent: Weak<Storage>,
    changed_entries: usize,
    fingerprint: u64,
    max_material: MaterialId,
    used_materials: OnceLock<Arc<[MaterialId]>>,
}

/// Persistent-map diffs describe physical allocation changes. Content equality
/// would hide independent equal bricks from shared-memory budget accounting.
#[derive(Clone, Debug)]
struct SharedChunk(Arc<Chunk>);
impl PartialEq for SharedChunk {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for SharedChunk {}
impl std::ops::Deref for SharedChunk {
    type Target = Chunk;
    fn deref(&self) -> &Chunk {
        &self.0
    }
}

/// A cheap snapshot: edits copy only map paths and changed eight-cell-wide bricks.
#[derive(Clone, Debug)]
pub struct Volume {
    dimensions: [u32; 3],
    limits: Limits,
    storage: Arc<Storage>,
    palette: Arc<MaterialPalette>,
    revision: u64,
}

impl PartialEq for Volume {
    fn eq(&self, other: &Self) -> bool {
        self.same_geometry(other)
            && (Arc::ptr_eq(&self.palette, &other.palette)
                || self.palette.materials == other.palette.materials)
    }
}
impl Eq for Volume {}

struct Edit<'a> {
    base: &'a Volume,
    chunks: OrdMap<[u32; 3], SharedChunk>,
    bytes: usize,
    occupied: usize,
    changed: bool,
    changed_entries: usize,
    fingerprint: u64,
    max_material: MaterialId,
}

impl<'a> Edit<'a> {
    fn new(base: &'a Volume) -> Self {
        Self {
            base,
            chunks: base.storage.chunks.clone(),
            bytes: base.storage.bytes,
            occupied: base.storage.occupied,
            changed: false,
            changed_entries: 0,
            fingerprint: base.storage.fingerprint,
            max_material: base.storage.max_material,
        }
    }
    fn replace(&mut self, key: [u32; 3], next: Option<Arc<Chunk>>) -> Result<(), String> {
        let old = self.chunks.get(&key);
        if old.map(|chunk| chunk.0.as_ref()) == next.as_deref() {
            return Ok(());
        }
        self.changed = true;
        self.changed_entries += 1;
        if let Some(old) = old {
            self.fingerprint ^= fingerprint(&(key, old.fingerprint));
            self.bytes -= old.resident_bytes();
            self.occupied -= old.occupied;
        }
        if let Some(next) = next {
            self.max_material = self.max_material.max(next.max_material);
            self.fingerprint ^= fingerprint(&(key, next.fingerprint));
            self.bytes += next.resident_bytes();
            self.occupied += next.occupied;
            self.chunks.insert(key, SharedChunk(next));
        } else {
            self.chunks.remove(&key);
        }
        if self.chunks.len() > self.base.limits.max_chunks
            || self.bytes + self.base.palette.bytes > self.base.limits.max_bytes
        {
            return Err(format!(
                "voxel edit needs {} chunks and {} resident bytes; limits are {} chunks and {} bytes",
                self.chunks.len(),
                self.bytes + self.base.palette.bytes,
                self.base.limits.max_chunks,
                self.base.limits.max_bytes
            ));
        }
        Ok(())
    }
    fn finish(self) -> Volume {
        if !self.changed {
            return self.base.clone();
        }
        Volume {
            dimensions: self.base.dimensions,
            limits: self.base.limits,
            storage: Arc::new(Storage {
                chunks: self.chunks,
                bytes: self.bytes,
                occupied: self.occupied,
                parent: Arc::downgrade(&self.base.storage),
                changed_entries: self.changed_entries,
                fingerprint: self.fingerprint,
                max_material: self.max_material,
                used_materials: OnceLock::new(),
            }),
            palette: self.base.palette.clone(),
            revision: revision(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct EncodedMaterial {
    color: u32,
    emission: f32,
    #[serde(default, skip_serializing_if = "OpticalProperties::is_default")]
    optics: OpticalProperties,
    #[serde(default, skip_serializing_if = "MaterialGeometry::is_default")]
    geometry: MaterialGeometry,
    faces: [Option<usize>; 6],
}

#[derive(Serialize, Deserialize)]
struct EncodedVolume {
    dimensions: [u32; 3],
    #[serde(deserialize_with = "decode_textures")]
    textures: Vec<Arc<Texture>>,
    #[serde(deserialize_with = "bounded::<_, _, {material::MAX_MATERIALS}>")]
    materials: Vec<EncodedMaterial>,
    #[serde(deserialize_with = "bounded::<_, _, MAX_CHUNKS>")]
    chunks: Vec<([u32; 3], Arc<Chunk>)>,
}

fn bounded<'de, D: Deserializer<'de>, T: Deserialize<'de>, const MAX: usize>(
    deserializer: D,
) -> Result<Vec<T>, D::Error> {
    struct Bounded<T, const MAX: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const MAX: usize> serde::de::Visitor<'de> for Bounded<T, MAX> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "at most {MAX} voxel entries")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|length| length > MAX) {
                return Err(serde::de::Error::custom(
                    "voxel entry count exceeds its limit",
                ));
            }
            let mut entries = Vec::new();
            while let Some(entry) = seq.next_element()? {
                if entries.len() == MAX {
                    return Err(serde::de::Error::custom(
                        "voxel entry count exceeds its limit",
                    ));
                }
                entries.push(entry);
            }
            Ok(entries)
        }
    }
    deserializer.deserialize_seq(Bounded::<T, MAX>(std::marker::PhantomData))
}

fn decode_textures<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Arc<Texture>>, D::Error> {
    struct Textures;
    impl<'de> serde::de::Visitor<'de> for Textures {
        type Value = Vec<Arc<Texture>>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("voxel textures within the resident byte limit")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut textures = Vec::new();
            let mut bytes = 0;
            while let Some(texture) = seq.next_element::<Arc<Texture>>()? {
                bytes += texture.pixels().len() + ENTRY_BYTES + size_of::<Texture>();
                if bytes > MAX_RESIDENT_BYTES {
                    return Err(serde::de::Error::custom(
                        "voxel textures exceed the resident byte limit",
                    ));
                }
                textures.push(texture);
            }
            Ok(textures)
        }
    }
    deserializer.deserialize_seq(Textures)
}

impl Serialize for Volume {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut texture_ids = OrdMap::new();
        let mut textures = Vec::new();
        let mut materials = Vec::with_capacity(self.palette.materials.len());
        for material in &self.palette.materials {
            let faces = material.faces().each_ref().map(|face| {
                face.as_ref().map(|texture| {
                    let identity = Arc::as_ptr(texture) as usize;
                    if let Some(index) = texture_ids.get(&identity) {
                        return *index;
                    }
                    let index = textures.len();
                    textures.push(texture.clone());
                    texture_ids.insert(identity, index);
                    index
                })
            });
            materials.push(EncodedMaterial {
                color: material.color(),
                emission: material.emission(),
                optics: *material.optics(),
                geometry: material.geometry(),
                faces,
            });
        }
        EncodedVolume {
            dimensions: self.dimensions,
            textures,
            materials,
            chunks: self
                .chunks()
                .map(|(key, chunk)| (key, chunk.clone()))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Volume {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = EncodedVolume::deserialize(deserializer)?;
        let mut volume = Self::new(encoded.dimensions).map_err(serde::de::Error::custom)?;
        for (index, material) in encoded.materials.into_iter().enumerate() {
            let mut faces = std::array::from_fn(|_| None);
            for (axis, face) in material.faces.into_iter().enumerate() {
                if let Some(id) = face {
                    faces[axis] = Some(
                        encoded
                            .textures
                            .get(id)
                            .ok_or_else(|| {
                                serde::de::Error::custom(format!(
                                    "voxel texture {id} does not exist"
                                ))
                            })?
                            .clone(),
                    );
                }
            }
            let material = Material::textured(material.color, faces)
                .and_then(|material_value| material_value.with_emission(material.emission))
                .and_then(|material_value| material_value.with_optics(material.optics))
                .and_then(|material_value| material_value.with_geometry(material.geometry))
                .map_err(serde::de::Error::custom)?;
            let (next, id) = volume
                .with_material(Arc::new(material))
                .map_err(serde::de::Error::custom)?;
            if id as usize != index + 1 {
                return Err(serde::de::Error::custom(
                    "voxel palette contains duplicate materials",
                ));
            }
            volume = next;
        }
        let mut chunks = OrdMap::new();
        let mut bytes = 0;
        let mut occupied = 0;
        let mut content_fingerprint = 0;
        let mut max_material = EMPTY;
        for (position, chunk) in encoded.chunks {
            let start = position.map(|axis| axis.checked_mul(CHUNK_EDGE));
            if (0..3).any(|axis| start[axis].is_none_or(|cell| cell >= volume.dimensions[axis])) {
                return Err(serde::de::Error::custom(
                    "voxel chunk lies outside the volume",
                ));
            }
            let start = start.map(Option::unwrap);
            for z in 0..CHUNK_EDGE {
                for y in 0..CHUNK_EDGE {
                    for x in 0..CHUNK_EDGE {
                        let material = chunk.get([x, y, z]);
                        if material != EMPTY
                            && (volume.material(material).is_none()
                                || (0..3).any(|axis| {
                                    start[axis] + [x, y, z][axis] >= volume.dimensions[axis]
                                }))
                        {
                            return Err(serde::de::Error::custom(
                                "voxel chunk references an absent material or contains occupied padding outside the volume",
                            ));
                        }
                    }
                }
            }
            bytes += chunk.resident_bytes();
            max_material = max_material.max(chunk.max_material);
            occupied += chunk.occupied();
            content_fingerprint ^= fingerprint(&(position, chunk.fingerprint));
            if chunks.insert(position, SharedChunk(chunk)).is_some() {
                return Err(serde::de::Error::custom(
                    "voxel volume contains duplicate chunk coordinates",
                ));
            }
            if chunks.len() > volume.limits.max_chunks
                || bytes + volume.palette.bytes > volume.limits.max_bytes
            {
                return Err(serde::de::Error::custom(
                    "voxel volume exceeds its resident chunk or byte budget",
                ));
            }
        }
        volume.storage = Arc::new(Storage {
            chunks,
            bytes,
            occupied,
            parent: Weak::new(),
            changed_entries: 0,
            fingerprint: content_fingerprint,
            max_material,
            used_materials: OnceLock::new(),
        });
        Ok(volume)
    }
}

impl Volume {
    pub fn new(dimensions: [u32; 3]) -> Result<Self, String> {
        Self::with_limits(dimensions, Limits::default())
    }

    pub fn with_limits(dimensions: [u32; 3], limits: Limits) -> Result<Self, String> {
        if limits.max_dimension > MAX_DIMENSION
            || limits.max_chunks > MAX_CHUNKS
            || limits.max_bytes > MAX_RESIDENT_BYTES
        {
            return Err("voxel limits may tighten, but cannot exceed the engine's dimension, chunk, or byte limits".into());
        }
        math::validate_dimensions(dimensions, limits.max_dimension)?;
        Ok(Self {
            dimensions,
            limits,
            storage: Arc::new(Storage {
                chunks: OrdMap::new(),
                bytes: 0,
                occupied: 0,
                parent: Weak::new(),
                changed_entries: 0,
                fingerprint: 0,
                max_material: EMPTY,
                used_materials: OnceLock::new(),
            }),
            palette: Arc::new(MaterialPalette::default()),
            revision: revision(),
        })
    }

    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn palette_revision(&self) -> u64 {
        self.palette.revision
    }
    pub fn resident_bytes(&self) -> usize {
        self.storage.bytes + self.palette.bytes
    }
    pub fn occupied_voxels(&self) -> usize {
        self.storage.occupied
    }
    pub fn chunk_count(&self) -> usize {
        self.storage.chunks.len()
    }
    /// Palette-independent geometry key; callers must confirm equal hashes with
    /// same_geometry before treating separately allocated grids as interchangeable.
    pub fn geometry_fingerprint(&self) -> u64 {
        self.storage.fingerprint
    }
    pub fn same_geometry(&self, other: &Self) -> bool {
        self.dimensions == other.dimensions
            && (Arc::ptr_eq(&self.storage, &other.storage)
                || (self.storage.occupied == other.storage.occupied
                    && self.storage.fingerprint == other.storage.fingerprint
                    && self.storage.chunks.len() == other.storage.chunks.len()
                    && self
                        .storage
                        .chunks
                        .iter()
                        .zip(other.storage.chunks.iter())
                        .all(|((key, chunk), (other_key, other_chunk))| {
                            key == other_key && chunk.0 == other_chunk.0
                        })))
    }
    pub fn same_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage) && Arc::ptr_eq(&self.palette, &other.palette)
    }
    pub fn storage_identity(&self) -> usize {
        Arc::as_ptr(&self.storage) as usize
    }
    /// Conservative new persistent-map storage since one direct immutable edit.
    /// A weak origin proves sharing without retaining an edit-history chain.
    pub fn storage_metadata_bytes_since(&self, previous: Option<&Self>) -> usize {
        if previous.is_some_and(|old| Arc::ptr_eq(&self.storage, &old.storage)) {
            return 0;
        }
        let bytes = if let Some(previous) =
            previous.filter(|old| self.storage.parent.as_ptr() == Arc::as_ptr(&old.storage))
        {
            math::copied_map_metadata_bytes(
                self.chunk_count(),
                previous.chunk_count(),
                self.storage.changed_entries,
            )
        } else {
            math::map_metadata_bytes(self.chunk_count())
        };
        // Reserve the intrinsic lazy material-index payload before it is built.
        // This remains a valid memory proof even when a later placement warms it.
        bytes.saturating_add(
            size_of::<Storage>()
                + 4 * size_of::<usize>()
                + self.storage.max_material as usize * size_of::<MaterialId>(),
        )
    }
    pub fn palette_identity(&self) -> usize {
        Arc::as_ptr(&self.palette) as usize
    }
    pub fn palette(&self) -> &Arc<MaterialPalette> {
        &self.palette
    }
    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Arc<Material>)> {
        self.palette.materials()
    }
    pub fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        self.palette.material(id)
    }

    /// Intern materials with the same palette implementation used by instance scenes.
    pub fn with_material(&self, material: Arc<Material>) -> Result<(Self, MaterialId), String> {
        let (palette, id) = self.palette.with_material(material)?;
        if Arc::ptr_eq(&palette, &self.palette) {
            return Ok((self.clone(), id));
        }
        if self.storage.bytes + palette.bytes > self.limits.max_bytes {
            return Err(format!(
                "voxel material exceeds the {} byte resident budget",
                self.limits.max_bytes
            ));
        }
        let mut volume = self.clone();
        volume.palette = palette;
        volume.revision = revision();
        Ok((volume, id))
    }

    pub fn fill_color(&self, min: [u32; 3], max: [u32; 3], color: u32) -> Result<Self, String> {
        self.check_box(min, max)?;
        if (0..3).any(|axis| min[axis] == max[axis]) {
            return Ok(self.clone());
        }
        if color & 0xff == 0 {
            return self.fill_box(min, max, EMPTY);
        }
        let (volume, material) = self.with_material(Arc::new(Material::solid(color)?))?;
        volume.fill_box(min, max, material)
    }

    /// Work estimate used by the language's existing execution budget.
    pub fn fill_work(&self, min: [u32; 3], max: [u32; 3]) -> Result<usize, String> {
        self.check_box(min, max)?;
        Ok(math::fill_work(min, max))
    }

    pub fn chunks(&self) -> impl Iterator<Item = ([u32; 3], &Arc<Chunk>)> {
        self.storage
            .chunks
            .iter()
            .map(|(position, chunk)| (*position, &chunk.0))
    }

    pub fn get(&self, position: [u32; 3]) -> Result<u32, String> {
        self.check_position(position)?;
        Ok(self.cell(position))
    }

    pub fn set(&self, position: [u32; 3], color: u32) -> Result<Self, String> {
        self.check_position(position)?;
        self.fill_box(position, position.map(|axis| axis + 1), color)
    }

    /// Fill a half-open region with an existing material, or zero to erase.
    pub fn fill_box(&self, min: [u32; 3], max: [u32; 3], color: u32) -> Result<Self, String> {
        if color != EMPTY && self.material(color).is_none() {
            return Err(format!(
                "voxel material {color} does not exist in this volume's palette"
            ));
        }
        self.check_box(min, max)?;
        if (0..3).any(|axis| min[axis] == max[axis]) {
            return Ok(self.clone());
        }
        if color == EMPTY && min == [0; 3] && max == self.dimensions {
            if self.storage.chunks.is_empty() {
                return Ok(self.clone());
            }
            let mut volume = self.clone();
            volume.storage = Arc::new(Storage {
                chunks: OrdMap::new(),
                bytes: 0,
                occupied: 0,
                parent: Arc::downgrade(&self.storage),
                changed_entries: self.storage.chunks.len(),
                fingerprint: 0,
                max_material: EMPTY,
                used_materials: OnceLock::new(),
            });
            volume.revision = revision();
            return Ok(volume);
        }
        let first = min.map(|axis| axis / CHUNK_EDGE);
        let last = max.map(|axis| (axis - 1) / CHUNK_EDGE);
        let keys = if color == EMPTY {
            self.storage
                .chunks
                .keys()
                .filter(|key| {
                    (0..3).all(|axis| key[axis] >= first[axis] && key[axis] <= last[axis])
                })
                .copied()
                .collect::<Vec<_>>()
        } else {
            let count = (0..3)
                .map(|axis| u64::from(last[axis] - first[axis] + 1))
                .product::<u64>();
            if count > self.limits.max_chunks as u64 {
                return Err(format!(
                    "voxel fill touches {count} chunks; the limit is {}",
                    self.limits.max_chunks
                ));
            }
            let mut keys = Vec::with_capacity(count as usize);
            for z in first[2]..=last[2] {
                for y in first[1]..=last[1] {
                    for x in first[0]..=last[0] {
                        keys.push([x, y, z]);
                    }
                }
            }
            keys
        };
        let mut edit = Edit::new(self);
        let mut uniform = None;
        for key in keys {
            let base = key.map(|axis| axis * CHUNK_EDGE);
            let start = std::array::from_fn::<_, 3, _>(|axis| min[axis].saturating_sub(base[axis]));
            let end =
                std::array::from_fn::<_, 3, _>(|axis| (max[axis] - base[axis]).min(CHUNK_EDGE));
            let old = edit.chunks.get(&key);
            let next = if start == [0; 3] && end == [CHUNK_EDGE; 3] {
                (color != EMPTY).then(|| {
                    uniform
                        .get_or_insert_with(|| Arc::new(Chunk::solid(color)))
                        .clone()
                })
            } else {
                let mut cells = old.map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                for z in start[2]..end[2] {
                    for y in start[1]..end[1] {
                        for x in start[0]..end[0] {
                            cells[index([x, y, z])] = color;
                        }
                    }
                }
                Chunk::from_cells(cells).map(Arc::new)
            };
            edit.replace(key, next)?;
        }
        Ok(edit.finish())
    }

    fn check_placement(&self, child: &Self, origin: [i32; 3]) -> Result<(), String> {
        if (0..3).any(|axis| {
            origin[axis] < 0
                || origin[axis] as u64 + u64::from(child.dimensions[axis])
                    > u64::from(self.dimensions[axis])
        }) {
            return Err(format!(
                "voxel child dimensions {:?} at {origin:?} do not fit parent dimensions {:?}",
                child.dimensions, self.dimensions
            ));
        }
        Ok(())
    }

    /// Bound native placement work before copying bricks. Aligned uniform data
    /// uses the brick fast path; unaligned data may touch eight destination bricks.
    pub fn placement_work(&self, child: &Self, origin: [i32; 3]) -> Result<usize, String> {
        self.check_placement(child, origin)?;
        if child.occupied_voxels() == 0 {
            return Ok(0);
        }
        let aligned = origin.into_iter().all(|axis| axis % CHUNK_EDGE as i32 == 0);
        let mut work = child.palette.materials.len().saturating_mul(7);
        for (_, chunk) in child.chunks() {
            let scan = if chunk.uniform().is_some() {
                1
            } else {
                CHUNK_VOXELS
            };
            let copies = if aligned { scan } else { CHUNK_VOXELS * 8 };
            work = work.saturating_add(scan).saturating_add(copies);
        }
        Ok(work)
    }

    pub fn used_materials(&self) -> &Arc<[MaterialId]> {
        self.storage.used_materials.get_or_init(|| {
            let mut used = [false; material::MAX_MATERIALS + 1];
            for (_, chunk) in self.chunks() {
                match &chunk.cells {
                    Cells::Uniform(material) => used[*material as usize] = true,
                    Cells::Dense(cells) => {
                        for material in cells.iter() {
                            used[*material as usize] = true;
                        }
                    }
                }
            }
            used.iter()
                .enumerate()
                .skip(1)
                .take(self.storage.max_material as usize)
                .filter_map(|(id, used)| used.then_some(id as MaterialId))
                .collect::<Vec<_>>()
                .into()
        })
    }
    /// Source-local IDs mapped to an already-composed world's palette. Empty or
    /// unused source IDs remain zero; no palette or geometry is copied here.
    pub fn material_remap(&self, source: &Self) -> Result<Arc<[MaterialId]>, String> {
        self.palette.material_remap(source)
    }

    fn placement_palette(&self, child: &Self) -> Result<(Self, Arc<[MaterialId]>, bool), String> {
        if Arc::ptr_eq(&self.palette, &child.palette) {
            return Ok((self.clone(), Arc::from([]), true));
        }
        let (palette, remap) = self.palette.with_volume(child)?;
        if self.storage.bytes + palette.bytes > self.limits.max_bytes {
            return Err("voxel placement palette exceeds the volume resident byte budget".into());
        }
        let mut parent = self.clone();
        if !Arc::ptr_eq(&parent.palette, &palette) {
            parent.palette = palette;
            parent.revision = revision();
        }
        let identity = remap
            .iter()
            .enumerate()
            .all(|(id, mapped)| *mapped == EMPTY || *mapped as usize == id);
        Ok((parent, remap, identity))
    }

    /// Overlay occupied child cells. Empty child space does not erase the parent.
    pub fn place(&self, child: &Self, origin: [i32; 3]) -> Result<Self, String> {
        self.check_placement(child, origin)?;
        if child.occupied_voxels() == 0 {
            return Ok(self.clone());
        }
        let origin = origin.map(|axis| axis as u32);
        let (parent, remap, identity) = self.placement_palette(child)?;
        let mut edit = Edit::new(&parent);
        if origin.into_iter().all(|axis| axis % CHUNK_EDGE == 0) {
            let shift = origin.map(|axis| axis / CHUNK_EDGE);
            let mut uniform = OrdMap::<MaterialId, Arc<Chunk>>::new();
            for (key, source) in child.chunks() {
                let target = std::array::from_fn(|axis| key[axis] + shift[axis]);
                if let Some(id) = source.uniform() {
                    let mapped = if identity { id } else { remap[id as usize] };
                    let chunk = if id == mapped {
                        source.clone()
                    } else if let Some(chunk) = uniform.get(&mapped) {
                        chunk.clone()
                    } else {
                        let chunk = Arc::new(Chunk::solid(mapped));
                        uniform.insert(mapped, chunk.clone());
                        chunk
                    };
                    edit.replace(target, Some(chunk))?;
                    continue;
                }
                if identity && !edit.chunks.contains_key(&target) {
                    edit.replace(target, Some(source.clone()))?;
                    continue;
                }
                let mut cells = edit
                    .chunks
                    .get(&target)
                    .map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                for (target, source) in cells.iter_mut().zip(source.values()) {
                    if source != EMPTY {
                        *target = if identity {
                            source
                        } else {
                            remap[source as usize]
                        };
                    }
                }
                edit.replace(target, Chunk::from_cells(cells).map(Arc::new))?;
            }
        } else {
            let mut targets = std::collections::BTreeSet::new();
            for (key, _) in child.chunks() {
                let start =
                    std::array::from_fn::<_, 3, _>(|axis| key[axis] * CHUNK_EDGE + origin[axis]);
                let end = std::array::from_fn::<_, 3, _>(|axis| {
                    ((key[axis] + 1) * CHUNK_EDGE).min(child.dimensions[axis]) + origin[axis]
                });
                for z in start[2] / CHUNK_EDGE..=(end[2] - 1) / CHUNK_EDGE {
                    for y in start[1] / CHUNK_EDGE..=(end[1] - 1) / CHUNK_EDGE {
                        for x in start[0] / CHUNK_EDGE..=(end[0] - 1) / CHUNK_EDGE {
                            targets.insert([x, y, z]);
                        }
                    }
                }
                if targets.len() > self.limits.max_chunks {
                    return Err(format!(
                        "voxel placement touches more than {} chunks",
                        self.limits.max_chunks
                    ));
                }
            }
            for key in targets {
                let mut cells = edit
                    .chunks
                    .get(&key)
                    .map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                let base = key.map(|axis| axis * CHUNK_EDGE);
                let start =
                    std::array::from_fn::<_, 3, _>(|axis| origin[axis].saturating_sub(base[axis]));
                let end = std::array::from_fn::<_, 3, _>(|axis| {
                    (origin[axis] + child.dimensions[axis])
                        .saturating_sub(base[axis])
                        .min(CHUNK_EDGE)
                });
                for z in start[2]..end[2] {
                    for y in start[1]..end[1] {
                        for x in start[0]..end[0] {
                            let local = [x, y, z];
                            let source =
                                std::array::from_fn(|axis| base[axis] + local[axis] - origin[axis]);
                            let material = child.cell(source);
                            if material != EMPTY {
                                cells[index(local)] = if identity {
                                    material
                                } else {
                                    remap[material as usize]
                                };
                            }
                        }
                    }
                }
                edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
            }
        }
        Ok(edit.finish())
    }

    /// Visit only changed persistent-map branches; no linked revision history is retained.
    pub fn visit_changes(
        &self,
        previous: &Self,
        mut visit: impl FnMut([u32; 3], Option<&Arc<Chunk>>),
    ) -> Result<usize, String> {
        if self.dimensions != previous.dimensions {
            return Err("voxel dimensions changed; rebuild the GPU page table".into());
        }
        let mut count = 0;
        for change in previous.storage.chunks.diff(&self.storage.chunks) {
            match change {
                DiffItem::Add(key, chunk)
                | DiffItem::Update {
                    new: (key, chunk), ..
                } => visit(*key, Some(&chunk.0)),
                DiffItem::Remove(key, _) => visit(*key, None),
            }
            count += 1;
        }
        Ok(count)
    }

    fn check_position(&self, position: [u32; 3]) -> Result<(), String> {
        if (0..3).any(|axis| position[axis] >= self.dimensions[axis]) {
            return Err(format!(
                "voxel position {position:?} is outside dimensions {:?}",
                self.dimensions
            ));
        }
        Ok(())
    }

    fn check_box(&self, min: [u32; 3], max: [u32; 3]) -> Result<(), String> {
        for axis in 0..3 {
            if min[axis] > max[axis] || max[axis] > self.dimensions[axis] {
                return Err(format!(
                    "voxel box {} range {}..{} is outside 0..{}",
                    ["x", "y", "z"][axis],
                    min[axis],
                    max[axis],
                    self.dimensions[axis]
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn cell(&self, position: [u32; 3]) -> u32 {
        self.storage
            .chunks
            .get(&position.map(|axis| axis / CHUNK_EDGE))
            .map_or(EMPTY, |chunk| {
                chunk.get(position.map(|axis| axis % CHUNK_EDGE))
            })
    }
}

fn index([x, y, z]: [u32; 3]) -> usize {
    (x + CHUNK_EDGE * (y + CHUNK_EDGE * z)) as usize
}

pub(crate) fn revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        value.checked_add(1)
    })
    .expect("voxel revision counter overflowed")
}
