mod atlas;
mod instances;
pub(crate) use instances::{INSTANCE_WORDS, synchronize_instances};

use crate::{Limits, RenderStats, UNIFORM_BYTES};
use sanim_voxel::{CHUNK_EDGE, CHUNK_VOXELS, Chunk, Material, Texture, Volume};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, Weak},
};

const UNIFORM_PAGE: u32 = 0x8000_0000;
const WORD_BYTES: u64 = size_of::<u32>() as u64;
const CHUNK_BYTES: u64 = CHUNK_VOXELS as u64 * WORD_BYTES;
pub(crate) const MATERIAL_WORDS: usize = 20;

#[derive(Clone, Copy)]
pub(crate) struct Context<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub layout: &'a wgpu::BindGroupLayout,
    pub uniforms: [&'a wgpu::Buffer; 2],
    pub limits: Limits,
}

struct GeometryUpload {
    pages: Arc<BTreeMap<usize, u32>>,
    page_count: usize,
    voxels: Vec<u32>,
    capacity: u32,
    slots: BTreeMap<[u32; 3], u32>,
    metadata: Vec<u32>,
    metadata_capacity: usize,
}

enum Source {
    Volume(Volume),
    Instances(instances::State),
}
impl Source {
    fn revision(&self) -> u64 {
        match self {
            Self::Volume(volume) => volume.revision(),
            Self::Instances(state) => state.scene.revision(),
        }
    }
    fn palette(&self) -> &Arc<sanim_voxel::MaterialPalette> {
        match self {
            Self::Volume(volume) => volume.palette(),
            Self::Instances(state) => state.scene.palette(),
        }
    }
    fn same_palette(&self, palette: &Arc<sanim_voxel::MaterialPalette>) -> bool {
        Arc::ptr_eq(self.palette(), palette) || self.palette().materials().eq(palette.materials())
    }
    fn dimensions(&self) -> [u32; 3] {
        match self {
            Self::Volume(volume) => volume.dimensions(),
            Self::Instances(state) => state.scene.dimensions(),
        }
    }
}

pub(crate) struct Resident {
    source: Source,
    pub(crate) has_transmission: bool,
    pub(crate) has_foliage: bool,
    pages: wgpu::Buffer,
    metadata: wgpu::Buffer,
    voxels: wgpu::Buffer,
    materials: wgpu::Buffer,
    _atlas: wgpu::Texture,
    atlas_view: wgpu::TextureView,
    atlas_textures: Vec<Weak<Texture>>,
    sampler: wgpu::Sampler,
    pub bind_group: wgpu::BindGroup,
    slots: BTreeMap<[u32; 3], u32>,
    free: Vec<u32>,
    next: u32,
    capacity: u32,
    bytes: u64,
    palette_bytes: u64,
}

impl Resident {
    fn retiring_palette_bytes(&self, upload: &MaterialUpload) -> u64 {
        if upload.same_atlas(self) {
            self.materials.size()
        } else {
            self.palette_bytes
        }
    }
}

struct MaterialUpload {
    has_transmission: bool,
    has_foliage: bool,
    words: Vec<u32>,
    textures: Vec<Arc<Texture>>,
    atlas: atlas::Plan,
    bytes: u64,
}

impl MaterialUpload {
    fn prepare<'a>(
        materials: impl IntoIterator<Item = (u32, &'a Arc<Material>)>,
    ) -> Result<Self, String> {
        let mut words = vec![0; MATERIAL_WORDS];
        let mut textures = Vec::<Arc<Texture>>::new();
        let mut has_transmission = false;
        let mut has_foliage = false;
        let mut indices = BTreeMap::new();
        for (id, material) in materials {
            if id as usize != words.len() / MATERIAL_WORDS {
                return Err("voxel material IDs must be consecutive starting at one".into());
            }
            words.push(material.color());
            for face in material.faces() {
                let Some(texture) = face else {
                    words.push(u32::MAX);
                    continue;
                };
                let key = Arc::as_ptr(texture) as usize;
                let index = *indices.entry(key).or_insert_with(|| {
                    let index = textures.len() as u32;
                    let cutout = texture.has_cutout();
                    textures.push(texture.clone());
                    index | if cutout { UNIFORM_PAGE } else { 0 }
                });
                words.push(index);
            }
            words.push(material.emission().to_bits());
            let optics = material.optics();
            has_transmission |= optics.transmission > 0.0;
            has_foliage |= material.geometry() == sanim_voxel::MaterialGeometry::Cross;
            words.extend([
                optics.transmission.to_bits(),
                optics.ior.to_bits(),
                optics.roughness.to_bits(),
                material.geometry() as u32,
            ]);
            // Beer–Lambert coefficient: attenuation_color is linear transmittance
            // after attenuation_distance voxel units, independently of surface tint.
            words.extend(optics.absorption().map(f32::to_bits));
            words.push(0);
            words.extend([
                optics.wave_strength.to_bits(),
                optics.wave_length.to_bits(),
                optics.wave_speed.to_bits(),
                0,
            ]);
        }
        let width = textures.first().map_or(1, |texture| texture.width());
        let height = textures.first().map_or(1, |texture| texture.height());
        if textures
            .iter()
            .any(|texture| texture.width() != width || texture.height() != height)
        {
            return Err("voxel texture pack tiles must have matching dimensions; resize the pack to one tile size".into());
        }
        let atlas = atlas::Plan::new(width, height, textures.len().max(1))?;
        // Include queued words, their CPU vector and a possible endian encoding,
        // texture references and the one current weak atlas proof before generation.
        let metadata = (words.capacity() + 2 * words.len()) as u64 * WORD_BYTES
            + (textures.capacity() * size_of::<Arc<Texture>>()
                + textures.len() * size_of::<Weak<Texture>>()
                + size_of::<Self>()) as u64;
        atlas.validate_temporary(metadata, sanim_voxel::MAX_RESIDENT_BYTES as u64)?;
        let bytes = (words.len() as u64 * WORD_BYTES)
            .checked_add(atlas.gpu_bytes)
            .ok_or("voxel material/atlas byte count overflowed")?;
        Ok(Self {
            has_transmission,
            has_foliage,
            words,
            textures,
            atlas,
            bytes,
        })
    }

    fn validate_device(&self, device: &wgpu::Device) -> Result<(), String> {
        if self.atlas.dimensions[0].max(self.atlas.dimensions[1])
            > device.limits().max_texture_dimension_2d
            || self.textures.len() as u32 > device.limits().max_texture_array_layers
        {
            return Err(
                "voxel texture pack exceeds the GPU texture size or array-layer limit".into(),
            );
        }
        Ok(())
    }

    fn same_atlas(&self, resident: &Resident) -> bool {
        resident.atlas_textures.len() == self.textures.len()
            && resident
                .atlas_textures
                .iter()
                .zip(&self.textures)
                .all(|(old, texture)| old.as_ptr() == Arc::as_ptr(texture))
    }

    fn create_materials(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Buffer {
        let materials = storage_buffer(
            device,
            "Voxel materials",
            self.words.len() as u64 * WORD_BYTES,
        );
        queue.write_buffer(&materials, 0, &bytes(&self.words));
        materials
    }

    fn create_atlas(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let [width, height] = self.atlas.dimensions;
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Voxel per-face texture tiles"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: self.textures.len().max(1) as u32,
            },
            mip_level_count: self.atlas.levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let white = [255; 4];
        for layer in 0..self.textures.len().max(1) {
            let pixels = self
                .textures
                .get(layer)
                .map_or(white.as_slice(), |texture| texture.pixels().as_ref());
            self.atlas
                .upload_levels(pixels, |level, [width, height], pixels| {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &atlas,
                            mip_level: level,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: layer as u32,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        pixels,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: Some(height),
                        },
                        wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                    );
                });
        }
        let view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        (atlas, view)
    }
}

pub(crate) fn synchronize(
    context: Context<'_>,
    resident: &mut Option<Resident>,
    volume: &Volume,
) -> Result<RenderStats, String> {
    let Context {
        device,
        queue,
        limits,
        ..
    } = context;
    if resident.as_ref().is_some_and(|state| {
        matches!(state.source, Source::Volume(_)) && state.source.revision() == volume.revision()
    }) {
        if resident.as_ref().expect("resident revision checked").bytes > limits.max_gpu_bytes {
            return Err("voxel volume plus output targets exceed the GPU byte budget; reduce output resolution or voxel data".into());
        }
        return Ok(RenderStats {
            resident_bytes: resident.as_ref().expect("resident revision checked").bytes,
            ..Default::default()
        });
    }
    let dimensions = volume.dimensions();
    let mut changes = Vec::<([u32; 3], Option<Arc<Chunk>>)>::new();
    let mut dense_count;
    if let Some(state) = resident.as_ref().filter(|state| {
        matches!(state.source, Source::Volume(_)) && state.source.dimensions() == dimensions
    }) {
        dense_count = state.slots.len();
        let Source::Volume(previous) = &state.source else {
            unreachable!("volume cache checked")
        };
        volume.visit_changes(previous, |coordinate, chunk| {
            let was_dense = state.slots.contains_key(&coordinate);
            // CPU budgets deliberately diff allocation identities. GPU page contents
            // can remain identical when a newly allocated uniform brick keeps its ID.
            if !was_dense
                && let Some(material) = chunk.and_then(|chunk| chunk.uniform())
                && previous
                    .get(coordinate.map(|axis| axis * CHUNK_EDGE))
                    .is_ok_and(|previous| previous == material)
            {
                return;
            }
            dense_count -= usize::from(was_dense);
            dense_count += usize::from(chunk.is_some_and(|chunk| chunk.uniform().is_none()));
            changes.push((coordinate, chunk.cloned()));
        })?;
    } else {
        dense_count = volume
            .chunks()
            .filter(|(_, chunk)| chunk.uniform().is_none())
            .count();
    }
    let capacity = dense_count
        .max(1)
        .checked_next_power_of_two()
        .and_then(|count| u32::try_from(count).ok())
        .ok_or("voxel dense-brick count exceeds GPU address space")?;
    let mut rebuild = resident.as_ref().is_none_or(|state| {
        !matches!(state.source, Source::Volume(_))
            || state.source.dimensions() != dimensions
            || capacity > state.capacity
            || capacity * 4 < state.capacity
    });
    let page_dimensions = dimensions.map(|dimension| dimension.div_ceil(CHUNK_EDGE));
    let page_count = page_dimensions
        .iter()
        .try_fold(1u64, |count, dimension| {
            count.checked_mul(u64::from(*dimension))
        })
        .ok_or("voxel GPU page-table dimensions overflowed")?;
    let capacity = if rebuild {
        capacity
    } else {
        resident
            .as_ref()
            .expect("resident exists without rebuild")
            .capacity
    };
    let voxel_bytes = u64::from(capacity) * CHUNK_BYTES;
    let material_upload = if rebuild
        || resident
            .as_ref()
            .is_none_or(|state| !state.source.same_palette(volume.palette()))
    {
        Some(MaterialUpload::prepare(volume.materials())?)
    } else {
        None
    };
    if let Some(upload) = &material_upload {
        upload.validate_device(device)?;
    }
    let palette_bytes = material_upload.as_ref().map_or_else(
        || {
            resident
                .as_ref()
                .expect("unchanged palette has resident state")
                .palette_bytes
        },
        |upload| upload.bytes,
    );
    let total_bytes = validate_sizes(
        context,
        [page_count * WORD_BYTES, voxel_bytes, WORD_BYTES],
        palette_bytes,
    )?;
    // Palette replacement may coexist briefly with its previous allocations.
    // If that would cross the byte cap, rebuild after retiring the old state instead.
    if !rebuild
        && material_upload.is_some()
        && resident.as_ref().is_some_and(|state| {
            total_bytes
                + state.retiring_palette_bytes(
                    material_upload.as_ref().expect("changed palette prepared"),
                )
                > limits.max_gpu_bytes
        })
    {
        rebuild = true;
    }
    if rebuild {
        // The caller has drained all previous submissions before a version change.
        *resident = None;
        let materials = material_upload.expect("rebuilt volume has prepared materials");
        let mut page_words = BTreeMap::new();
        let mut voxel_words = Vec::with_capacity(dense_count * CHUNK_VOXELS);
        let mut slots = BTreeMap::new();
        for (coordinate, chunk) in volume.chunks() {
            let page = if let Some(material) = chunk.uniform() {
                UNIFORM_PAGE | material
            } else {
                let slot = slots.len() as u32;
                slots.insert(coordinate, slot);
                voxel_words.extend(chunk.values());
                slot + 1
            };
            page_words.insert(page_index(coordinate, page_dimensions), page);
        }
        let geometry = GeometryUpload {
            pages: Arc::new(page_words),
            page_count: page_count as usize,
            voxels: voxel_words,
            capacity,
            slots,
            metadata: vec![0],
            metadata_capacity: 1,
        };
        let (state, stats) = create(
            context,
            Source::Volume(volume.clone()),
            materials,
            geometry,
            total_bytes,
        );
        *resident = Some(state);
        return Ok(stats);
    }
    let state = resident
        .as_mut()
        .expect("incremental synchronization has resident state");
    let mut stats = RenderStats {
        resident_bytes: total_bytes,
        ..Default::default()
    };
    if let Some(materials) = material_upload {
        replace_palette(context, state, &materials, &mut stats);
    }
    for (coordinate, chunk) in &changes {
        if chunk.as_ref().is_none_or(|chunk| chunk.uniform().is_some())
            && let Some(slot) = state.slots.remove(coordinate)
        {
            state.free.push(slot);
        }
    }
    for (coordinate, chunk) in changes {
        let page = if let Some(chunk) = chunk {
            if let Some(material) = chunk.uniform() {
                UNIFORM_PAGE | material
            } else {
                let slot = *state.slots.entry(coordinate).or_insert_with(|| {
                    state.free.pop().unwrap_or_else(|| {
                        let slot = state.next;
                        state.next += 1;
                        slot
                    })
                });
                debug_assert!(slot < state.capacity);
                queue.write_buffer(
                    &state.voxels,
                    u64::from(slot) * CHUNK_BYTES,
                    &bytes(&chunk.values()),
                );
                stats.uploaded_chunks += 1;
                stats.upload_bytes += CHUNK_BYTES;
                slot + 1
            }
        } else {
            0
        };
        queue.write_buffer(
            &state.pages,
            page_index(coordinate, page_dimensions) as u64 * WORD_BYTES,
            &page.to_le_bytes(),
        );
        stats.updated_pages += 1;
        stats.upload_bytes += WORD_BYTES;
    }
    state.source = Source::Volume(volume.clone());
    state.bytes = total_bytes;
    Ok(stats)
}

/// Validate the same material/texture plan used by GPU synchronization, without a device.
pub fn validate_materials<'a>(
    materials: impl IntoIterator<Item = (sanim_voxel::MaterialId, &'a Arc<Material>)>,
) -> Result<(), String> {
    MaterialUpload::prepare(materials).map(drop)
}

fn storage_buffer(device: &wgpu::Device, label: &'static str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.max(WORD_BYTES),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 6],
    atlas: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Voxel resident resources"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffers[0].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: buffers[1].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: buffers[2].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: buffers[3].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(atlas),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: buffers[4].as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: buffers[5].as_entire_binding(),
            },
        ],
    })
}

fn page_index(coordinate: [u32; 3], dimensions: [u32; 3]) -> usize {
    (coordinate[0] + dimensions[0] * (coordinate[1] + dimensions[1] * coordinate[2])) as usize
}

pub(crate) fn bytes(words: &[u32]) -> Cow<'_, [u8]> {
    if cfg!(target_endian = "little") {
        Cow::Borrowed(bytemuck::cast_slice(words))
    } else {
        Cow::Owned(words.iter().flat_map(|word| word.to_le_bytes()).collect())
    }
}

impl Resident {
    pub(crate) fn revision(&self) -> u64 {
        self.source.revision()
    }
    pub(crate) fn shader_features(&self) -> usize {
        usize::from(self.has_transmission)
            | (usize::from(self.has_foliage) << 1)
            | (usize::from(matches!(self.source, Source::Instances(_))) << 2)
    }
}

fn validate_sizes(
    context: Context<'_>,
    buffers: [u64; 3],
    palette_bytes: u64,
) -> Result<u64, String> {
    let binding_limit = context
        .device
        .limits()
        .max_storage_buffer_binding_size
        .min(context.device.limits().max_buffer_size);
    if buffers.into_iter().any(|bytes| bytes > binding_limit) {
        return Err("voxel page table, dense bricks, or instance metadata exceed the GPU storage-buffer binding limit".into());
    }
    let bytes = buffers
        .into_iter()
        .try_fold(
            palette_bytes + UNIFORM_BYTES + crate::lights::UNIFORM_BYTES,
            u64::checked_add,
        )
        .ok_or("voxel GPU resource sizes overflowed")?;
    if bytes > context.limits.max_gpu_bytes {
        return Err(format!(
            "voxel GPU resources require {bytes} bytes, exceeding the {} byte budget",
            context.limits.max_gpu_bytes
        ));
    }
    Ok(bytes)
}

fn create(
    context: Context<'_>,
    source: Source,
    material: MaterialUpload,
    geometry: GeometryUpload,
    total_bytes: u64,
) -> (Resident, RenderStats) {
    let Context {
        device,
        queue,
        layout,
        uniforms,
        ..
    } = context;
    let materials = material.create_materials(device, queue);
    let (atlas, atlas_view) = material.create_atlas(device, queue);
    let pages = storage_buffer(
        device,
        "Voxel sparse brick page table",
        geometry.page_count as u64 * WORD_BYTES,
    );
    let voxels = storage_buffer(
        device,
        "Voxel shared dense bricks",
        u64::from(geometry.capacity) * CHUNK_BYTES,
    );
    let metadata = storage_buffer(
        device,
        "Voxel instance metadata",
        geometry.metadata_capacity as u64 * WORD_BYTES,
    );
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Voxel pixel texture sampling"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Nearest,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let bind_group = bind(
        device,
        layout,
        [
            uniforms[0],
            &pages,
            &voxels,
            &materials,
            uniforms[1],
            &metadata,
        ],
        &atlas_view,
        &sampler,
    );
    write_runs(
        queue,
        &pages,
        0,
        geometry.pages.iter().map(|(index, word)| (*index, *word)),
    );
    if !geometry.voxels.is_empty() {
        queue.write_buffer(&voxels, 0, &bytes(&geometry.voxels));
    }
    queue.write_buffer(&metadata, 0, &bytes(&geometry.metadata));
    let stats = RenderStats {
        uploaded_chunks: geometry.voxels.len() / CHUNK_VOXELS,
        updated_pages: geometry.pages.len(),
        upload_bytes: (geometry.pages.len() + geometry.voxels.len() + geometry.metadata.len())
            as u64
            * WORD_BYTES
            + material.bytes,
        buffer_allocations: 4,
        texture_allocations: 1,
        resident_bytes: total_bytes,
        ..Default::default()
    };
    (
        Resident {
            source,
            has_transmission: material.has_transmission,
            has_foliage: material.has_foliage,
            pages,
            voxels,
            metadata,
            materials,
            _atlas: atlas,
            atlas_view,
            atlas_textures: material.textures.iter().map(Arc::downgrade).collect(),
            sampler,
            bind_group,
            next: geometry.slots.len() as u32,
            slots: geometry.slots,
            free: Vec::new(),
            capacity: geometry.capacity,
            bytes: total_bytes,
            palette_bytes: material.bytes,
        },
        stats,
    )
}

fn replace_palette(
    context: Context<'_>,
    state: &mut Resident,
    material: &MaterialUpload,
    stats: &mut RenderStats,
) {
    state.materials = material.create_materials(context.device, context.queue);
    let same_atlas = material.same_atlas(state);
    if !same_atlas {
        let (atlas, view) = material.create_atlas(context.device, context.queue);
        state._atlas = atlas;
        state.atlas_view = view;
        state.atlas_textures = material.textures.iter().map(Arc::downgrade).collect();
        stats.texture_allocations += 1;
        stats.upload_bytes += material.atlas.gpu_bytes;
    }
    state.palette_bytes = material.bytes;
    state.has_transmission = material.has_transmission;
    state.has_foliage = material.has_foliage;
    state.bind_group = bind(
        context.device,
        context.layout,
        [
            context.uniforms[0],
            &state.pages,
            &state.voxels,
            &state.materials,
            context.uniforms[1],
            &state.metadata,
        ],
        &state.atlas_view,
        &state.sampler,
    );
    stats.buffer_allocations += 1;
    stats.upload_bytes += material.words.len() as u64 * WORD_BYTES;
}

/// Coalesce sorted changed words into the minimum contiguous queue uploads.
fn write_runs(
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    base: usize,
    changes: impl IntoIterator<Item = (usize, u32)>,
) -> (usize, u64) {
    let mut start = 0;
    let mut run = Vec::new();
    let mut words = 0;
    for (index, value) in changes {
        if !run.is_empty() && index != start + run.len() {
            queue.write_buffer(buffer, (base + start) as u64 * WORD_BYTES, &bytes(&run));
            words += run.len();
            run.clear();
        }
        if run.is_empty() {
            start = index;
        }
        run.push(value);
    }
    if !run.is_empty() {
        queue.write_buffer(buffer, (base + start) as u64 * WORD_BYTES, &bytes(&run));
        words += run.len();
    }
    (words, words as u64 * WORD_BYTES)
}
