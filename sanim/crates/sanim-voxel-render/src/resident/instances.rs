//! Raw asset geometry is shared independently of integer placement and palette.
use super::*;
use sanim_voxel::InstanceScene;

type AssetKey = ([u32; 3], usize);
const HEADER_WORDS: usize = 4;
const ASSET_WORDS: usize = 8;
pub(crate) const INSTANCE_WORDS: usize = 8;

#[derive(Clone)]
struct Asset {
    _volume: Arc<Volume>,
    row: u32,
}
pub(super) struct State {
    pub(super) scene: InstanceScene,
    assets: BTreeMap<AssetKey, Asset>,
    prefix: Arc<[u32]>,
    metadata_capacity: usize,
    pages: Arc<BTreeMap<usize, u32>>,
    metadata_tail: Arc<[u32]>,
}
struct Assets {
    entries: BTreeMap<AssetKey, Asset>,
    prefix: Arc<[u32]>,
    voxels: Vec<u32>,
}

fn unique_assets(scene: &InstanceScene) -> BTreeMap<AssetKey, Arc<Volume>> {
    let mut assets = BTreeMap::new();
    for instance in scene.instances() {
        assets
            .entry((
                instance.volume.dimensions(),
                instance.volume.storage_identity(),
            ))
            .or_insert_with(|| instance.volume.clone());
    }
    assets
}

fn prepare_assets(
    unique: BTreeMap<AssetKey, Arc<Volume>>,
    maximum_bytes: u64,
) -> Result<Assets, String> {
    let mut canonical = Vec::<Arc<Volume>>::new();
    let mut fingerprints = BTreeMap::<([u32; 3], u64), Vec<usize>>::new();
    let mut aliases = Vec::with_capacity(unique.len());
    for (key, volume) in unique {
        let candidates = fingerprints
            .entry((volume.dimensions(), volume.geometry_fingerprint()))
            .or_default();
        let index = if let Some(index) = candidates
            .iter()
            .copied()
            .find(|index| canonical[*index].same_geometry(&volume))
        {
            index
        } else {
            let index = canonical.len();
            canonical.push(volume.clone());
            candidates.push(index);
            index
        };
        aliases.push((key, volume, index));
    }
    let mut prefix = vec![0; HEADER_WORDS + canonical.len() * ASSET_WORDS];
    let mut voxels = Vec::new();
    let mut chunks = BTreeMap::new();
    for (index, volume) in canonical.into_iter().enumerate() {
        let dimensions = volume.dimensions();
        let pages = dimensions.map(|dimension| dimension.div_ceil(CHUNK_EDGE));
        let count = pages.into_iter().map(u64::from).product::<u64>();
        let page_start = prefix.len();
        if (page_start as u64 + count + voxels.len() as u64) * WORD_BYTES > maximum_bytes {
            return Err("voxel asset page tables exceed the GPU byte budget".into());
        }
        prefix.resize(page_start + count as usize, 0);
        for (coordinate, chunk) in volume.chunks() {
            let page = if let Some(material) = chunk.uniform() {
                UNIFORM_PAGE | material
            } else {
                let key = Arc::as_ptr(chunk) as usize;
                if let Some(slot) = chunks.get(&key) {
                    *slot + 1
                } else {
                    let slot = (voxels.len() / CHUNK_VOXELS) as u32;
                    if (prefix.len() as u64 + voxels.len() as u64 + CHUNK_VOXELS as u64)
                        * WORD_BYTES
                        > maximum_bytes
                    {
                        return Err(
                            "shared voxel asset geometry exceeds the GPU byte budget".into()
                        );
                    }
                    voxels.extend(chunk.values());
                    chunks.insert(key, slot);
                    slot + 1
                }
            };
            prefix[page_start + page_index(coordinate, pages)] = page;
        }
        let row = HEADER_WORDS + index * ASSET_WORDS;
        prefix[row..row + ASSET_WORDS].copy_from_slice(&[
            dimensions[0],
            dimensions[1],
            dimensions[2],
            page_start as u32,
            pages[0],
            pages[1],
            0,
            0,
        ]);
    }
    let entries = aliases
        .into_iter()
        .map(|(key, volume, index)| {
            (
                key,
                Asset {
                    _volume: volume,
                    row: (HEADER_WORDS + index * ASSET_WORDS) as u32,
                },
            )
        })
        .collect();
    prefix[0] = prefix.len() as u32;
    Ok(Assets {
        entries,
        prefix: prefix.into(),
        voxels,
    })
}

fn match_assets(
    unique: &BTreeMap<AssetKey, Arc<Volume>>,
    previous: &BTreeMap<AssetKey, Asset>,
) -> Option<BTreeMap<AssetKey, Asset>> {
    if unique.keys().eq(previous.keys()) {
        return Some(previous.clone());
    }
    let mut candidates = BTreeMap::<([u32; 3], u64), Vec<&Asset>>::new();
    let mut old_rows = std::collections::BTreeSet::new();
    for asset in previous.values() {
        if old_rows.insert(asset.row) {
            candidates
                .entry((
                    asset._volume.dimensions(),
                    asset._volume.geometry_fingerprint(),
                ))
                .or_default()
                .push(asset);
        }
    }
    let mut used_rows = std::collections::BTreeSet::new();
    let mut result = BTreeMap::new();
    for (key, volume) in unique {
        let asset = if let Some(asset) = previous.get(key) {
            asset
        } else {
            candidates
                .get(&(volume.dimensions(), volume.geometry_fingerprint()))?
                .iter()
                .copied()
                .find(|asset| asset._volume.same_geometry(volume))?
        };
        used_rows.insert(asset.row);
        result.insert(
            *key,
            Asset {
                _volume: volume.clone(),
                row: asset.row,
            },
        );
    }
    // Compact on removal instead of retaining historical asset allocations.
    (used_rows == old_rows).then_some(result)
}

struct Placement {
    pages: Arc<BTreeMap<usize, u32>>,
    page_count: usize,
    metadata: Vec<u32>,
}
fn prepare_placement(
    scene: &InstanceScene,
    assets: &BTreeMap<AssetKey, Asset>,
    prefix: &[u32],
    maximum_bytes: u64,
) -> Result<Placement, String> {
    let page_dimensions = scene
        .dimensions()
        .map(|dimension| dimension.div_ceil(CHUNK_EDGE));
    let count = page_dimensions.into_iter().map(u64::from).product::<u64>();
    if count * WORD_BYTES > maximum_bytes {
        return Err("voxel instance page table exceeds the GPU byte budget".into());
    }
    let mut pages = BTreeMap::new();
    let mut metadata = vec![0; scene.instances().len() * INSTANCE_WORDS];
    let base = prefix.len();
    let mut remaps = BTreeMap::<Arc<[u32]>, u32>::new();
    let mut remap_identities = BTreeMap::<usize, u32>::new();
    for (index, instance) in scene.instances().iter().enumerate() {
        let key = (
            instance.volume.dimensions(),
            instance.volume.storage_identity(),
        );
        let asset = assets.get(&key).expect("unique source asset prepared");
        let identity = Arc::as_ptr(&instance.materials) as *const u32 as usize;
        let remap = *remap_identities.entry(identity).or_insert_with(|| {
            *remaps.entry(instance.materials.clone()).or_insert_with(|| {
                let start = (base + metadata.len()) as u32;
                metadata.extend(instance.materials.iter().copied());
                start
            })
        });
        let row = index * INSTANCE_WORDS;
        metadata[row..row + INSTANCE_WORDS].copy_from_slice(&[
            instance.origin[0],
            instance.origin[1],
            instance.origin[2],
            asset.row,
            remap,
            instance.materials.len() as u32,
            0,
            0,
        ]);
        if (base + metadata.len()) as u64 * WORD_BYTES > maximum_bytes {
            return Err("voxel instance palette remaps exceed the GPU byte budget".into());
        }
    }
    // The model builds and validates this one spatial index; the renderer only
    // packs its ordered lists into the device's bounded storage representation.
    let buckets = scene.buckets();
    for bucket in &buckets.buckets {
        let last = (bucket.start + bucket.len - 1) as usize;
        let instance = &scene.instances()[buckets.indices[last] as usize];
        let asset = &assets[&(
            instance.volume.dimensions(),
            instance.volume.storage_identity(),
        )];
        let row = asset.row as usize;
        let local_pages = [
            prefix[row + 4],
            prefix[row + 5],
            instance.volume.dimensions()[2].div_ceil(CHUNK_EDGE),
        ];
        if let Some(material) = crate::math::instances::uniform_brick(
            instance.origin,
            instance.volume.dimensions(),
            bucket.coordinate,
            |coordinate| {
                let page = prefix[prefix[row + 3] as usize + page_index(coordinate, local_pages)];
                (page & UNIFORM_PAGE != 0).then_some(page & !UNIFORM_PAGE)
            },
        ) {
            // The final painter covers the whole brick: lower instances cannot
            // contribute. Reuse the ordinary uniform-page and medium-skip path.
            pages.insert(
                page_index(bucket.coordinate, page_dimensions),
                UNIFORM_PAGE | instance.materials[material as usize],
            );
            continue;
        }
        if ((base + metadata.len()) as u64 + 1 + u64::from(bucket.len)) * WORD_BYTES > maximum_bytes
        {
            return Err("voxel instance buckets exceed the GPU byte budget".into());
        }
        pages.insert(
            page_index(bucket.coordinate, page_dimensions),
            (base + metadata.len()) as u32 + 1,
        );
        metadata.push(bucket.len);
        let start = bucket.start as usize;
        metadata.extend_from_slice(&buckets.indices[start..start + bucket.len as usize]);
    }
    Ok(Placement {
        pages: Arc::new(pages),
        page_count: count as usize,
        metadata,
    })
}

pub(crate) fn synchronize_instances(
    context: Context<'_>,
    resident: &mut Option<Resident>,
    scene: &InstanceScene,
) -> Result<RenderStats, String> {
    if let Some(state) = resident.as_ref()
        && matches!(state.source, Source::Instances(_))
        && state.source.revision() == scene.revision()
    {
        if state.bytes > context.limits.max_gpu_bytes {
            return Err("voxel instances plus output targets exceed the GPU byte budget".into());
        }
        return Ok(RenderStats {
            resident_bytes: state.bytes,
            ..Default::default()
        });
    }
    let unique = unique_assets(scene);
    let reusable = resident.as_ref().and_then(|state| {
        let Source::Instances(previous) = &state.source else {
            return None;
        };
        match_assets(&unique, &previous.assets)
    });
    let material = if resident
        .as_ref()
        .is_none_or(|state| !state.source.same_palette(scene.palette()))
        || reusable.is_none()
    {
        let material = MaterialUpload::prepare(scene.materials())?;
        material.validate_device(context.device)?;
        Some(material)
    } else {
        None
    };
    if reusable.is_none() {
        let assets = prepare_assets(unique, context.limits.max_gpu_bytes)?;
        let placement = prepare_placement(
            scene,
            &assets.entries,
            &assets.prefix,
            context.limits.max_gpu_bytes,
        )?;
        let capacity = (assets.voxels.len() / CHUNK_VOXELS)
            .max(1)
            .next_power_of_two() as u32;
        let metadata_capacity = (assets.prefix.len() + placement.metadata.len())
            .max(1)
            .next_power_of_two();
        let material = material.expect("new instances have prepared palette");
        let total_bytes = validate_sizes(
            context,
            [
                placement.page_count as u64 * WORD_BYTES,
                u64::from(capacity) * CHUNK_BYTES,
                metadata_capacity as u64 * WORD_BYTES,
            ],
            material.bytes,
        )?;
        let state = State {
            scene: scene.clone(),
            assets: assets.entries,
            prefix: assets.prefix,
            metadata_capacity,
            pages: placement.pages.clone(),
            metadata_tail: placement.metadata.clone().into(),
        };
        let mut metadata = state.prefix.to_vec();
        metadata.extend(placement.metadata);
        let geometry = GeometryUpload {
            pages: placement.pages,
            page_count: placement.page_count,
            voxels: assets.voxels,
            capacity,
            slots: BTreeMap::new(),
            metadata,
            metadata_capacity,
        };
        // All old users were retired by Renderer::prepare before any replacement.
        *resident = None;
        let (created, stats) = create(
            context,
            Source::Instances(state),
            material,
            geometry,
            total_bytes,
        );
        *resident = Some(created);
        return Ok(stats);
    }
    let previous = resident.as_ref().expect("reusable instance state exists");
    let Source::Instances(cache) = &previous.source else {
        unreachable!("instance state checked")
    };
    let placement = prepare_placement(
        scene,
        reusable.as_ref().expect("reusable geometry checked"),
        &cache.prefix,
        context.limits.max_gpu_bytes,
    )?;
    let needed = (cache.prefix.len() + placement.metadata.len())
        .max(1)
        .next_power_of_two();
    let metadata_capacity =
        if needed > cache.metadata_capacity || needed * 4 < cache.metadata_capacity {
            needed
        } else {
            cache.metadata_capacity
        };
    let page_resize = previous.source.dimensions() != scene.dimensions();
    let metadata_resize = metadata_capacity != cache.metadata_capacity;
    let palette_bytes = material
        .as_ref()
        .map_or(previous.palette_bytes, |material| material.bytes);
    let total_bytes = validate_sizes(
        context,
        [
            placement.page_count as u64 * WORD_BYTES,
            u64::from(previous.capacity) * CHUNK_BYTES,
            metadata_capacity as u64 * WORD_BYTES,
        ],
        palette_bytes,
    )?;
    let retiring = u64::from(metadata_resize) * cache.metadata_capacity as u64 * WORD_BYTES
        + if page_resize {
            previous.pages.size()
        } else {
            0
        }
        + material
            .as_ref()
            .map_or(0, |upload| previous.retiring_palette_bytes(upload));
    if total_bytes
        .checked_add(retiring)
        .is_none_or(|peak| peak > context.limits.max_gpu_bytes)
    {
        // Avoid transient resource overcommit. Retiring the current buffers costs
        // a cold asset upload, while retaining all logical snapshots is cheap.
        *resident = None;
        return synchronize_instances(context, resident, scene);
    }
    let state = resident.as_mut().expect("instance resources present");
    let mut stats = RenderStats {
        resident_bytes: total_bytes,
        ..Default::default()
    };
    let Source::Instances(cache) = &mut state.source else {
        unreachable!("instance state checked")
    };
    if page_resize {
        state.pages = storage_buffer(
            context.device,
            "Voxel instance world page table",
            placement.page_count as u64 * WORD_BYTES,
        );
        stats.buffer_allocations += 1;
    }
    if metadata_resize {
        let metadata = storage_buffer(
            context.device,
            "Voxel instance metadata",
            metadata_capacity as u64 * WORD_BYTES,
        );
        // The immutable asset page-table prefix is copied on the GPU. Growing or
        // shrinking placement metadata never reuploads the underlying geometry.
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Resize voxel instance metadata"),
            });
        encoder.copy_buffer_to_buffer(
            &state.metadata,
            0,
            &metadata,
            0,
            cache.prefix.len() as u64 * WORD_BYTES,
        );
        let submission = context.queue.submit([encoder.finish()]);
        context
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: None,
            })
            .map_err(|error| format!("resize bounded voxel instance metadata: {error}"))?;
        state.metadata = metadata;
        cache.metadata_capacity = metadata_capacity;
        stats.buffer_allocations += 1;
    }
    let (page_words, page_bytes) = if page_resize {
        write_runs(
            context.queue,
            &state.pages,
            0,
            placement
                .pages
                .iter()
                .map(|(index, value)| (*index, *value)),
        )
    } else {
        write_runs(
            context.queue,
            &state.pages,
            0,
            page_changes(&cache.pages, &placement.pages),
        )
    };
    let prefix = cache.prefix.len();
    let (_, metadata_bytes) = write_runs(
        context.queue,
        &state.metadata,
        prefix,
        placement
            .metadata
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, value)| {
                metadata_resize || cache.metadata_tail.get(*index) != Some(value)
            }),
    );
    stats.updated_pages = page_words;
    stats.upload_bytes = page_bytes + metadata_bytes;
    cache.pages = placement.pages;
    cache.metadata_tail = placement.metadata.into();
    cache.scene = scene.clone();
    cache.assets = reusable.expect("reusable geometry checked");
    if let Some(material) = material {
        replace_palette(context, state, &material, &mut stats);
    } else if page_resize || metadata_resize {
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
    }
    state.bytes = total_bytes;
    Ok(stats)
}

fn page_changes<'a>(
    previous: &'a BTreeMap<usize, u32>,
    next: &'a BTreeMap<usize, u32>,
) -> impl Iterator<Item = (usize, u32)> + 'a {
    let mut previous = previous.iter().peekable();
    let mut next = next.iter().peekable();
    std::iter::from_fn(move || {
        loop {
            match (previous.peek(), next.peek()) {
                (None, None) => return None,
                (Some(_), None) => {
                    let (index, _) = previous.next().expect("previous page available");
                    return Some((*index, 0));
                }
                (None, Some(_)) => {
                    let (index, value) = next.next().expect("next page available");
                    return Some((*index, *value));
                }
                (Some((old_index, _)), Some((new_index, _))) => match old_index.cmp(new_index) {
                    std::cmp::Ordering::Less => {
                        let (index, _) = previous.next().expect("previous page available");
                        return Some((*index, 0));
                    }
                    std::cmp::Ordering::Greater => {
                        let (index, value) = next.next().expect("next page available");
                        return Some((*index, *value));
                    }
                    std::cmp::Ordering::Equal => {
                        let (_, old) = previous.next().expect("previous page available");
                        let (index, value) = next.next().expect("next page available");
                        if old != value {
                            return Some((*index, *value));
                        }
                    }
                },
            }
        }
    })
}
