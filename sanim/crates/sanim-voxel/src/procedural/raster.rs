//! Chunk-batched field rasterization. Published volumes stay immutable.
mod surface;
use super::{Field, MAX_PROCEDURAL_WORK, MAX_SURFACE_POINTS};
use crate::{CHUNK_EDGE, CHUNK_VOXELS, Chunk, EMPTY, Edit, MaterialId, Volume, index};
use std::sync::Arc;
pub use surface::SurfacePoints;

/// A bounded two-byte-per-column plan; it is temporary native data, not a DSL List.
pub struct Heightmap {
    dimensions: [u32; 3],
    heights: Vec<u16>,
    work: usize,
}

fn checked_work(work: usize) -> Result<usize, String> {
    if work > MAX_PROCEDURAL_WORK {
        Err(format!(
            "procedural operation requires {work} work units, exceeding {MAX_PROCEDURAL_WORK}; reduce its bounds or field complexity"
        ))
    } else {
        Ok(work)
    }
}

impl Heightmap {
    pub fn sample_work(field: &Field, dimensions: [u32; 3]) -> Result<usize, String> {
        if dimensions
            .iter()
            .any(|axis| *axis == 0 || *axis > crate::MAX_DIMENSION)
        {
            return Err(format!(
                "heightmap dimensions must each be within 1..={}",
                crate::MAX_DIMENSION
            ));
        }
        checked_work(
            (dimensions[0] as usize)
                .saturating_mul(dimensions[2] as usize)
                .saturating_mul(field.sample_work()),
        )
    }
    pub fn new(field: &Field, dimensions: [u32; 3]) -> Result<Self, String> {
        Self::sample_work(field, dimensions)?;
        let mut sampler = field.sampler();
        let mut heights = Vec::with_capacity(dimensions[0] as usize * dimensions[2] as usize);
        for z in 0..dimensions[2] {
            for x in 0..dimensions[0] {
                heights.push(
                    sampler
                        .sample([f64::from(x) + 0.5, 0.0, f64::from(z) + 0.5])?
                        .floor()
                        .clamp(0.0, f64::from(dimensions[1])) as u16,
                );
            }
        }
        let mut plan = Self {
            dimensions,
            heights,
            work: 0,
        };
        for z in 0..dimensions[2].div_ceil(CHUNK_EDGE) {
            for x in 0..dimensions[0].div_ceil(CHUNK_EDGE) {
                let (min, max) = plan.column_range(x, z);
                for y in 0..u32::from(max).div_ceil(CHUNK_EDGE) {
                    plan.work += if plan.full_chunk([x, y, z], min) {
                        1
                    } else {
                        CHUNK_VOXELS
                    };
                }
            }
        }
        checked_work(plan.work)?;
        Ok(plan)
    }
    pub fn work(&self) -> usize {
        self.work
    }
    fn height(&self, x: u32, z: u32) -> u32 {
        u32::from(self.heights[(x + self.dimensions[0] * z) as usize])
    }
    fn column_range(&self, x: u32, z: u32) -> (u16, u16) {
        let mut min = u16::MAX;
        let mut max = 0;
        for z in z * CHUNK_EDGE..((z + 1) * CHUNK_EDGE).min(self.dimensions[2]) {
            for x in x * CHUNK_EDGE..((x + 1) * CHUNK_EDGE).min(self.dimensions[0]) {
                let height = self.height(x, z) as u16;
                min = min.min(height);
                max = max.max(height);
            }
        }
        (min, max)
    }
    fn full_chunk(&self, key: [u32; 3], min_height: u16) -> bool {
        (key[0] + 1) * CHUNK_EDGE <= self.dimensions[0]
            && (key[2] + 1) * CHUNK_EDGE <= self.dimensions[2]
            && (key[1] + 1) * CHUNK_EDGE <= u32::from(min_height)
    }
    pub fn apply(self, volume: &Volume, material: MaterialId) -> Result<Volume, String> {
        if volume.dimensions() != self.dimensions {
            return Err("heightmap plan dimensions do not match the target volume".into());
        }
        if material == EMPTY || volume.material(material).is_none() {
            return Err(
                "heightmap requires a nonempty material registered in its target volume".into(),
            );
        }
        let mut edit = Edit::new(volume);
        let uniform = Arc::new(Chunk::solid(material));
        for z in 0..self.dimensions[2].div_ceil(CHUNK_EDGE) {
            for x in 0..self.dimensions[0].div_ceil(CHUNK_EDGE) {
                let (min, max) = self.column_range(x, z);
                for y in 0..u32::from(max).div_ceil(CHUNK_EDGE) {
                    let key = [x, y, z];
                    if self.full_chunk(key, min) {
                        edit.replace(key, Some(uniform.clone()))?;
                        continue;
                    }
                    let mut cells = edit
                        .chunks
                        .get(&key)
                        .map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                    let base = key.map(|axis| axis * CHUNK_EDGE);
                    for z in 0..CHUNK_EDGE.min(self.dimensions[2] - base[2]) {
                        for x in 0..CHUNK_EDGE.min(self.dimensions[0] - base[0]) {
                            let height = self
                                .height(base[0] + x, base[2] + z)
                                .saturating_sub(base[1])
                                .min(CHUNK_EDGE);
                            for y in 0..height {
                                cells[index([x, y, z])] = material;
                            }
                        }
                    }
                    edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
                }
            }
        }
        Ok(edit.finish())
    }
}

impl Volume {
    pub fn flood_work(&self, level: u32) -> Result<usize, String> {
        if level > self.dimensions[1] {
            return Err("flood level exceeds the volume height".into());
        }
        checked_work(self.dimensions[0] as usize * self.dimensions[2] as usize * level as usize)
    }
    /// Fill only empty cells below an integer waterline; existing terrain remains untouched.
    pub fn flood(&self, level: u32, material: MaterialId) -> Result<Self, String> {
        self.flood_work(level)?;
        if material == EMPTY || self.material(material).is_none() {
            return Err(
                "flood requires a nonempty material registered in its target volume".into(),
            );
        }
        let mut edit = Edit::new(self);
        let uniform = Arc::new(Chunk::solid(material));
        for z in 0..self.dimensions[2].div_ceil(CHUNK_EDGE) {
            for y in 0..level.div_ceil(CHUNK_EDGE) {
                for x in 0..self.dimensions[0].div_ceil(CHUNK_EDGE) {
                    let key = [x, y, z];
                    let old = self.storage.chunks.get(&key);
                    if old.is_some_and(|chunk| chunk.uniform().is_some()) {
                        continue;
                    }
                    let base = key.map(|axis| axis * CHUNK_EDGE);
                    let end = [self.dimensions[0], level, self.dimensions[2]];
                    let extent = std::array::from_fn::<_, 3, _>(|axis| {
                        CHUNK_EDGE.min(end[axis] - base[axis])
                    });
                    if old.is_none() && extent == [CHUNK_EDGE; 3] {
                        edit.replace(key, Some(uniform.clone()))?;
                        continue;
                    }
                    let mut cells = old.map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                    let mut changed = false;
                    for z in 0..extent[2] {
                        for y in 0..extent[1] {
                            for x in 0..extent[0] {
                                let cell = &mut cells[index([x, y, z])];
                                if *cell == EMPTY {
                                    *cell = material;
                                    changed = true;
                                }
                            }
                        }
                    }
                    if changed {
                        edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
                    }
                }
            }
        }
        Ok(edit.finish())
    }
    pub fn scatter_work(&self, field: &Field, step: u32) -> Result<usize, String> {
        self.surface_sampling_work(&SurfacePoints {
            step,
            density: Some(field),
            ..Default::default()
        })
    }
    /// Place one cell above sampled terrain columns using a continuous probability field.
    /// Native cutout cards reuse their material geometry; no layout node is allocated per blade.
    pub fn scatter(
        &self,
        field: &Field,
        material: MaterialId,
        seed: u64,
        step: u32,
    ) -> Result<Self, String> {
        self.scatter_work(field, step)?;
        if material == EMPTY || self.material(material).is_none() {
            return Err("scatter requires a nonempty material registered in the volume".into());
        }
        let mut changes = std::collections::BTreeMap::new();
        self.visit_surface_points(
            &SurfacePoints {
                step,
                density: Some(field),
                seed,
                ..Default::default()
            },
            self.dimensions[1].saturating_sub(1),
            |point| {
                let key = point.map(|axis| axis / CHUNK_EDGE);
                let cells = changes.entry(key).or_insert_with(|| {
                    self.storage
                        .chunks
                        .get(&key)
                        .map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values())
                });
                cells[index(point.map(|axis| axis % CHUNK_EDGE))] = material;
                if changes.len() > self.limits.max_chunks {
                    return Err(
                        "scatter exceeds the volume chunk budget; reduce density or increase step"
                            .into(),
                    );
                }
                Ok(())
            },
        )?;
        let mut edit = Edit::new(self);
        for (key, cells) in changes {
            edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
        }
        Ok(edit.finish())
    }

    /// Repeated assets remap their palette once, then retain shared aligned bricks.
    pub fn place_many(&self, child: &Self, positions: &[[i32; 3]]) -> Result<Self, String> {
        if positions.len() > MAX_SURFACE_POINTS {
            return Err(format!(
                "voxel placement batch exceeds {MAX_SURFACE_POINTS} instances"
            ));
        }
        let mut work = 0usize;
        for origin in positions {
            work = work.saturating_add(self.placement_work(child, *origin)?);
        }
        checked_work(work)?;
        if positions.is_empty() || child.occupied_voxels() == 0 {
            return Ok(self.clone());
        }
        let (mut parent, remap, identity) = self.placement_palette(child)?;
        let mut mapped = child.clone();
        mapped.palette = parent.palette.clone();
        if !identity {
            let mut edit = Edit::new(&mapped);
            let mut shared = std::collections::HashMap::new();
            for (key, source) in child.chunks() {
                let id = Arc::as_ptr(source) as usize;
                let next = if let Some(next) = shared.get(&id) {
                    Arc::clone(next)
                } else {
                    let cells = source.values().map(|value| {
                        if value == EMPTY {
                            EMPTY
                        } else {
                            remap[value as usize]
                        }
                    });
                    let next = Arc::new(
                        Chunk::from_cells(cells)
                            .expect("a nonempty source remains nonempty after palette mapping"),
                    );
                    shared.insert(id, next.clone());
                    next
                };
                edit.replace(key, Some(next))?;
            }
            mapped = edit.finish();
        }
        for origin in positions {
            parent = parent.place(&mapped, *origin)?;
        }
        Ok(parent)
    }

    pub fn density_work(
        &self,
        field: &Field,
        min: [u32; 3],
        max: [u32; 3],
    ) -> Result<usize, String> {
        self.check_box(min, max)?;
        checked_work(
            (0..3)
                .map(|axis| (max[axis] - min[axis]) as usize)
                .product::<usize>()
                .saturating_mul(field.sample_work() + 1),
        )
    }
    /// Fill where the scalar density is greater than or equal to the threshold.
    pub fn fill_density(
        &self,
        field: &Field,
        min: [u32; 3],
        max: [u32; 3],
        threshold: f64,
        material: MaterialId,
    ) -> Result<Self, String> {
        self.density_work(field, min, max)?;
        if !threshold.is_finite() {
            return Err("density threshold must be finite".into());
        }
        if material != EMPTY && self.material(material).is_none() {
            return Err("density material does not belong to its target volume".into());
        }
        if (0..3).any(|axis| min[axis] == max[axis]) {
            return Ok(self.clone());
        }
        let mut sampler = field.sampler();
        let mut edit = Edit::new(self);
        let first = min.map(|axis| axis / CHUNK_EDGE);
        let last = max.map(|axis| (axis - 1) / CHUNK_EDGE);
        for z in first[2]..=last[2] {
            for y in first[1]..=last[1] {
                for x in first[0]..=last[0] {
                    let key = [x, y, z];
                    let base = key.map(|axis| axis * CHUNK_EDGE);
                    let mut cells = edit
                        .chunks
                        .get(&key)
                        .map_or([EMPTY; CHUNK_VOXELS], |chunk| chunk.values());
                    let start =
                        std::array::from_fn::<_, 3, _>(|axis| min[axis].saturating_sub(base[axis]));
                    let end = std::array::from_fn::<_, 3, _>(|axis| {
                        (max[axis] - base[axis]).min(CHUNK_EDGE)
                    });
                    for z in start[2]..end[2] {
                        for y in start[1]..end[1] {
                            for x in start[0]..end[0] {
                                let point = [x, y, z];
                                if sampler.sample(std::array::from_fn(|axis| {
                                    f64::from(base[axis] + point[axis]) + 0.5
                                }))? >= threshold
                                {
                                    cells[index(point)] = material;
                                }
                            }
                        }
                    }
                    edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
                }
            }
        }
        Ok(edit.finish())
    }

    fn surface_heights(&self) -> Vec<u16> {
        let [width, height, depth] = self.dimensions();
        let cards = self
            .materials()
            .any(|(_, material)| material.geometry() == crate::MaterialGeometry::Cross);
        let supports_surface = |id| {
            id != EMPTY
                && (!cards
                    || self.material(id).is_some_and(|material| {
                        material.geometry() == crate::MaterialGeometry::Cube
                    }))
        };
        let mut heights = vec![0u16; width as usize * depth as usize];
        for (key, chunk) in self.chunks() {
            if chunk.uniform().is_some_and(|id| !supports_surface(id)) {
                continue;
            }
            let base = key.map(|axis| axis * CHUNK_EDGE);
            for z in 0..CHUNK_EDGE.min(depth - base[2]) {
                for x in 0..CHUNK_EDGE.min(width - base[0]) {
                    let at = (base[0] + x + width * (base[2] + z)) as usize;
                    let top = (base[1] + CHUNK_EDGE).min(height);
                    if u32::from(heights[at]) >= top {
                        continue;
                    }
                    let value = if chunk.uniform().is_some() {
                        top
                    } else {
                        (0..CHUNK_EDGE.min(height - base[1]))
                            .rev()
                            .find(|y| supports_surface(chunk.get([x, *y, z])))
                            .map_or(0, |y| base[1] + y + 1)
                    };
                    heights[at] = heights[at].max(value as u16);
                }
            }
        }
        heights
    }
    pub fn surface_work(&self) -> usize {
        self.chunks()
            .map(|(_, chunk)| {
                if chunk.uniform().is_some() {
                    (CHUNK_EDGE * CHUNK_EDGE) as usize
                } else {
                    CHUNK_VOXELS
                }
            })
            .sum::<usize>()
            .saturating_add(self.dimensions[0] as usize * self.dimensions[2] as usize)
            .saturating_add(self.palette.materials.len())
    }
    pub fn surface_paint_work(&self, mask: Option<&Field>) -> Result<usize, String> {
        checked_work(self.surface_work().saturating_add(mask.map_or(0, |field| {
            (self.dimensions[0] as usize)
                .saturating_mul(self.dimensions[2] as usize)
                .saturating_mul(field.sample_work())
        })))
    }
    /// Paint selected top columns without filling cavities; masks sample their top boundary.
    pub fn paint_surface(
        &self,
        material: MaterialId,
        depth: u32,
        mask: Option<&Field>,
    ) -> Result<Self, String> {
        if material == EMPTY || self.material(material).is_none() {
            return Err(
                "surface paint requires a nonempty material registered in its target volume".into(),
            );
        }
        if depth == 0 || depth > self.dimensions[1] {
            return Err("surface depth must be within 1..=volume height".into());
        }
        self.surface_paint_work(mask)?;
        let mut heights = self.surface_heights();
        let width = self.dimensions[0];
        if let Some(mask) = mask {
            let mut sampler = mask.sampler();
            for z in 0..self.dimensions[2] {
                for x in 0..width {
                    let top = &mut heights[(x + width * z) as usize];
                    if *top != 0
                        && sampler.sample([
                            f64::from(x) + 0.5,
                            f64::from(*top),
                            f64::from(z) + 0.5,
                        ])? <= 0.0
                    {
                        *top = 0;
                    }
                }
            }
        }
        let mut edit = Edit::new(self);
        for (key, old) in self.chunks() {
            let base = key.map(|axis| axis * CHUNK_EDGE);
            let mut relevant = false;
            for z in 0..CHUNK_EDGE.min(self.dimensions[2] - base[2]) {
                for x in 0..CHUNK_EDGE.min(width - base[0]) {
                    let top = u32::from(heights[(base[0] + x + width * (base[2] + z)) as usize]);
                    if top > base[1] && top.saturating_sub(depth) < base[1] + CHUNK_EDGE {
                        relevant = true;
                        break;
                    }
                }
                if relevant {
                    break;
                }
            }
            if !relevant {
                continue;
            }
            let mut cells = old.values();
            let mut changed = false;
            for z in 0..CHUNK_EDGE.min(self.dimensions[2] - base[2]) {
                for x in 0..CHUNK_EDGE.min(width - base[0]) {
                    let top = u32::from(heights[(base[0] + x + width * (base[2] + z)) as usize]);
                    let start = top.saturating_sub(depth).saturating_sub(base[1]);
                    let end = top.saturating_sub(base[1]).min(CHUNK_EDGE);
                    for y in start.min(CHUNK_EDGE)..end {
                        let at = index([x, y, z]);
                        if cells[at] != EMPTY && cells[at] != material {
                            cells[at] = material;
                            changed = true;
                        }
                    }
                }
            }
            if changed {
                edit.replace(key, Chunk::from_cells(cells).map(Arc::new))?;
            }
        }
        Ok(edit.finish())
    }
}
