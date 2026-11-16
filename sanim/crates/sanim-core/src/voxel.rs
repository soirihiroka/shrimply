//! Renderer-independent voxel views use the ordinary retained property graph.
use crate::{Fraction, Geometry, Number, Property, Type, Unit, Value, math};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};
pub mod camera;
pub mod config;
pub mod lights;

const MAX_LAYOUT_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LAYOUT_CACHE_ENTRIES: usize = 16;
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum LayoutMode {
    Flattened,
    Instanced,
}
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum LayoutKey {
    Recipe([u8; 32], LayoutMode),
    Assembly([u8; 32], LayoutMode),
}
#[derive(Clone)]
enum CachedLayout {
    RecipeAlias(LayoutKey),
    Resolved(Arc<sanim_voxel::layout::Resolved>, usize),
}
type LayoutCache = crate::cache::Cache<LayoutKey, CachedLayout, CachedLayout>;

fn cached_layout(
    cache: &mut LayoutCache,
    key: &LayoutKey,
) -> Option<(Arc<sanim_voxel::layout::Resolved>, usize)> {
    match cache.get(key)? {
        CachedLayout::Resolved(resolved, bytes) => Some((resolved, bytes)),
        CachedLayout::RecipeAlias(key) => match cache.get(&key)? {
            CachedLayout::Resolved(resolved, bytes) => Some((resolved, bytes)),
            CachedLayout::RecipeAlias(_) => {
                unreachable!("assembly cache entries contain resolved output")
            }
        },
    }
}
fn layout_cache() -> &'static Mutex<LayoutCache> {
    static CACHE: OnceLock<Mutex<LayoutCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LayoutCache::new(
            MAX_LAYOUT_CACHE_BYTES,
            MAX_LAYOUT_CACHE_ENTRIES,
        ))
    })
}

/// Memoize pure outputs using the same bounded cache for recipe aliases and
/// resolved assemblies. Fractional recipes with identical snapped placements
/// share one output; source trees, prepared plans, and GPU resources are excluded.
pub fn resolve_layout(
    layout: &Arc<sanim_voxel::layout::Node>,
) -> Result<Arc<sanim_voxel::layout::Resolved>, String> {
    resolve(layout, LayoutMode::Flattened)
}
pub fn resolve_instances_layout(
    layout: &Arc<sanim_voxel::layout::Node>,
) -> Result<Arc<sanim_voxel::layout::Resolved>, String> {
    resolve(layout, LayoutMode::Instanced)
}
fn resolve(
    layout: &Arc<sanim_voxel::layout::Node>,
    mode: LayoutMode,
) -> Result<Arc<sanim_voxel::layout::Resolved>, String> {
    let recipe = LayoutKey::Recipe(layout.fingerprint(), mode);
    if let Some((resolved, _)) = cached_layout(
        &mut layout_cache().lock().expect("voxel layout cache poisoned"),
        &recipe,
    ) {
        return Ok(resolved);
    }
    let plan = sanim_voxel::layout::prepare(layout)?;
    let fingerprint = plan.fingerprint();
    let assembly = LayoutKey::Assembly(fingerprint, mode);
    let existing = cached_layout(
        &mut layout_cache().lock().expect("voxel layout cache poisoned"),
        &assembly,
    );
    let (resolved, bytes) = match existing {
        Some(cached) => cached,
        None => {
            let resolved = Arc::new(match mode {
                LayoutMode::Flattened => plan.assemble()?,
                LayoutMode::Instanced => plan.assemble_instances()?,
            });
            let bytes = resolved
                .source
                .resident_bytes()
                .saturating_add(std::mem::size_of_val(resolved.as_ref()))
                .saturating_add(2 * std::mem::size_of_val(&assembly))
                .saturating_add(std::mem::size_of::<CachedLayout>())
                .saturating_add(
                    resolved
                        .placements
                        .iter()
                        .map(|placement| {
                            std::mem::size_of_val(placement)
                                + placement.path.len() * std::mem::size_of::<usize>()
                        })
                        .sum::<usize>(),
                );
            (resolved, bytes)
        }
    };
    let mut cache = layout_cache().lock().expect("voxel layout cache poisoned");
    // A concurrent pure solve may already have published the same assembly.
    let (resolved, bytes) = if let Some(existing) = cached_layout(&mut cache, &assembly) {
        existing
    } else {
        cache.insert(
            assembly,
            CachedLayout::Resolved(resolved.clone(), bytes),
            bytes,
        );
        (resolved, bytes)
    };
    let alias_bytes = 4 * std::mem::size_of::<LayoutKey>()
        + 2 * std::mem::size_of::<CachedLayout>()
        + 4 * std::mem::size_of::<usize>();
    if bytes.saturating_add(alias_bytes) <= MAX_LAYOUT_CACHE_BYTES
        && cached_layout(&mut cache, &assembly).is_some()
    {
        cache.insert(recipe, CachedLayout::RecipeAlias(assembly), alias_bytes);
    }
    Ok(resolved)
}

pub fn layout_cache_usage() -> (usize, usize) {
    layout_cache()
        .lock()
        .expect("voxel layout cache poisoned")
        .usage()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_spec: Option<Arc<camera::Spec>>,
    pub source: sanim_voxel::SceneSource,
    pub layout: Option<Arc<sanim_voxel::layout::Node>>,
    pub placements: Arc<[sanim_voxel::layout::Placed]>,
    pub camera: sanim_voxel::camera::CameraRig,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "sanim_voxel::deserialize_lights"
    )]
    pub lights: Option<Arc<[sanim_voxel::Light]>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "sanim_voxel::lighting::scene::deserialize_environment"
    )]
    pub environment: Option<sanim_voxel::Environment>,
    #[serde(default)]
    pub check_framing: bool,
    #[serde(default)]
    pub frame_margin: Fraction,
}

impl Scene {
    pub fn dimensions(&self) -> [u32; 3] {
        self.source.dimensions()
    }
    pub fn occupied_voxels(&self) -> Option<usize> {
        self.source.occupied_voxels()
    }
    pub fn occupied_voxels_upper_bound(&self) -> usize {
        self.source.occupied_voxels_upper_bound()
    }
    pub fn resident_bytes(&self) -> usize {
        self.source.resident_bytes()
    }
    pub fn revision(&self) -> u64 {
        self.source.revision()
    }
    pub fn materials(&self) -> impl Iterator<Item = (u32, &Arc<sanim_voxel::Material>)> {
        self.source.materials()
    }
    pub fn material(&self, id: u32) -> Option<&Arc<sanim_voxel::Material>> {
        self.source.material(id)
    }
    pub fn raycast(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
    ) -> Result<Option<sanim_voxel::math::Hit>, String> {
        self.source.raycast(origin, direction)
    }

    pub fn resolve_camera(&self, viewport: [u32; 2]) -> Result<sanim_voxel::Camera, String> {
        match &self.camera_spec {
            Some(camera) => camera.resolve(self.dimensions(), viewport),
            None => self.camera.resolve(self.dimensions(), viewport),
        }
    }
}

pub const INPUTS: &[(&str, Property)] = &[
    ("camera", Property::VoxelCamera),
    ("volume", Property::VoxelVolume),
    ("layout", Property::VoxelLayout),
    ("view", Property::VoxelView),
    ("yaw", Property::VoxelYaw),
    ("pitch", Property::VoxelPitch),
    ("zoom", Property::VoxelZoom),
    ("fov", Property::VoxelFov),
    ("check_framing", Property::VoxelCheckFraming),
    ("frame_margin", Property::VoxelFrameMargin),
    ("lights", Property::VoxelLights),
    ("environment", Property::VoxelEnvironment),
];

pub fn input_type(property: Property) -> Option<Type> {
    Some(match property {
        Property::VoxelCamera => camera::camera_type(),
        Property::VoxelLights => lights::lights_type(),
        Property::VoxelEnvironment => lights::environment_type(),
        Property::VoxelVolume => Type::VoxelGrid,
        Property::VoxelLayout => Type::VoxelLayout,
        Property::VoxelView => Type::union(
            sanim_voxel::View::ALL
                .into_iter()
                .map(|view| Type::StringLiteral(view.name().into())),
        )
        .expect("voxel view choices"),
        Property::VoxelYaw | Property::VoxelPitch | Property::VoxelFov => {
            Type::Numeric(Unit::Angle)
        }
        Property::VoxelZoom => Type::Numeric(Unit::Number),
        Property::VoxelCheckFraming => Type::Bool,
        Property::VoxelFrameMargin => Type::Numeric(Unit::Percent),
        _ => return None,
    })
}

pub fn property(geometry: &Geometry, property: Property) -> Result<Value, String> {
    let Geometry::Voxel(scene) = geometry else {
        return Err("voxel camera properties require a VoxelScene element".into());
    };
    let (value, unit) = match property {
        Property::VoxelCamera=>return scene.camera_spec.as_ref().map(|camera|camera.value()).ok_or_else(||"this scene uses its fitted view; supply camera=Camera(...) to inspect a camera recipe".into()),
        Property::VoxelLights => {
            return lights::lights_value(
                scene
                    .lights
                    .as_deref()
                    .unwrap_or(&[sanim_voxel::Light::Sun(Default::default())]),
            );
        }
        Property::VoxelEnvironment => {
            return lights::environment_value(&scene.environment.unwrap_or(
                sanim_voxel::Environment {
                    visible: false,
                    ..Default::default()
                },
            ));
        }
        Property::VoxelVolume => return scene.source.flatten().map(Value::VoxelGrid),
        Property::VoxelLayout => {
            return scene
                .layout
                .as_ref()
                .map(|layout| Value::VoxelLayout(layout.clone()))
                .ok_or_else(|| "VoxelScene has a direct volume; no layout was supplied".into());
        }
        Property::VoxelView => return Ok(Value::String(scene.camera.view.to_string().into())),
        Property::VoxelCheckFraming => return Ok(Value::Bool(scene.check_framing)),
        Property::VoxelFrameMargin => {
            return Ok(Value::Number(Number {
                value: scene.frame_margin,
                unit: Unit::Percent,
            }));
        }
        Property::VoxelYaw => (scene.camera.yaw_degrees, Unit::Angle),
        Property::VoxelPitch => (scene.camera.pitch_degrees, Unit::Angle),
        Property::VoxelZoom => (scene.camera.zoom, Unit::Number),
        Property::VoxelFov => (scene.camera.vertical_fov_degrees.unwrap_or(0.), Unit::Angle),
        _ => return Err("not a voxel property".into()),
    };
    Ok(Value::Number(Number {
        value: math::from_f64(f64::from(value))?,
        unit,
    }))
}

pub fn set(geometry: &mut Geometry, property: Property, value: Value) -> Result<(), String> {
    let Geometry::Voxel(scene) = geometry else {
        return Err("voxel camera properties require a VoxelScene element".into());
    };
    let scene = Arc::make_mut(scene);
    match property {
        Property::VoxelCamera => {
            let camera = Arc::new(camera::Spec::new(&value)?);
            if scene.camera_spec.as_deref() != Some(camera.as_ref()) {
                scene.camera_spec = Some(camera);
            }
        }
        Property::VoxelLights => {
            let lights = lights::lights(&value)?;
            sanim_voxel::scene_light_parameters(
                &lights,
                scene.dimensions(),
                sanim_voxel::ShadowMode::Soft,
            )?;
            if scene.lights.as_deref() != Some(lights.as_ref()) {
                scene.lights = Some(lights);
            }
        }
        Property::VoxelEnvironment => scene.environment = Some(lights::environment(&value)?),
        Property::VoxelVolume => {
            let Value::VoxelGrid(volume) = value else {
                return Err("VoxelScene volume requires a VoxelGrid".into());
            };
            scene.source = sanim_voxel::SceneSource::Volume(volume);
            scene.layout = None;
            scene.placements = Arc::from([]);
        }
        Property::VoxelLayout => {
            let Value::VoxelLayout(layout) = value else {
                return Err("VoxelScene layout requires a VoxelBox or VoxelPart".into());
            };
            if scene
                .layout
                .as_ref()
                .is_some_and(|old| Arc::ptr_eq(old, &layout) || old.same_recipe(&layout))
            {
                return Ok(());
            }
            let resolved = resolve_instances_layout(&layout)?;
            scene.source = resolved.source.clone();
            scene.placements = resolved.placements.clone();
            scene.layout = Some(layout);
        }
        Property::VoxelView => scene.camera.view = value.string()?.parse()?,
        Property::VoxelCheckFraming => {
            let Value::Bool(enabled) = value else {
                return Err("voxel check_framing requires Bool".into());
            };
            scene.check_framing = enabled;
        }
        Property::VoxelFrameMargin => {
            let number = value.number()?;
            if number.unit != Unit::Percent
                || !math::finite(&number.value)
                || number.value < Fraction::from(0)
                || number.value >= Fraction::new(1u64, 2u64)
            {
                return Err(
                    "voxel frame_margin requires a finite percentage between 0% and 50%".into(),
                );
            }
            scene.frame_margin = number.value;
        }
        Property::VoxelYaw | Property::VoxelPitch | Property::VoxelZoom | Property::VoxelFov => {
            let number = value.number()?;
            let expected = if property == Property::VoxelZoom {
                Unit::Number
            } else {
                Unit::Angle
            };
            if number.unit != expected || !math::finite(&number.value) {
                return Err(format!(
                    "voxel {property:?} requires a finite {}",
                    Type::Numeric(expected)
                ));
            }
            let scalar = math::to_f64(number.value)? as f32;
            if !scalar.is_finite() {
                return Err("voxel camera value exceeds the GPU numeric range".into());
            }
            match property {
                Property::VoxelYaw => scene.camera.yaw_degrees = scalar,
                Property::VoxelPitch => scene.camera.pitch_degrees = scalar,
                Property::VoxelZoom if scalar > 0. => scene.camera.zoom = scalar,
                Property::VoxelZoom => return Err("voxel zoom must be greater than zero".into()),
                Property::VoxelFov if number.value == Fraction::from(0) => {
                    scene.camera.vertical_fov_degrees = None;
                }
                Property::VoxelFov if scalar > 0. && scalar < 179. => {
                    scene.camera.vertical_fov_degrees = Some(scalar);
                }
                Property::VoxelFov => return Err("voxel fov must be 0deg for orthographic projection, or between 0deg and 179deg for perspective".into()),
                _ => unreachable!(),
            }
        }
        _ => return Err("not a voxel property".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sanim_voxel::layout::{Dimension, Node, Placement, Space};

    fn layout(volume: Arc<sanim_voxel::Volume>, offset: Fraction, wrapper: u32) -> Arc<Node> {
        let part = Arc::new(
            Node::part(
                volume,
                Placement {
                    absolute: true,
                    offset: [offset, Fraction::from(0), Fraction::from(0)],
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let inner = Arc::new(
            Node::space(Space {
                size: [
                    Dimension::Cells(wrapper),
                    Dimension::Cells(4),
                    Dimension::Cells(4),
                ],
                children: std::iter::once(part).collect(),
                ..Default::default()
            })
            .unwrap(),
        );
        Arc::new(
            Node::space(Space {
                size: [
                    Dimension::Cells(12),
                    Dimension::Cells(4),
                    Dimension::Cells(4),
                ],
                children: std::iter::once(inner).collect(),
                ..Default::default()
            })
            .unwrap(),
        )
    }

    #[test]
    fn snapped_layout_memo_shares_payload_but_invalidates_placement_metadata() {
        let source = Arc::new(
            sanim_voxel::Volume::new([4; 3])
                .unwrap()
                .fill_color([0; 3], [4; 3], 0x33aa55ff)
                .unwrap(),
        );
        let original =
            resolve_layout(&layout(source.clone(), Fraction::new(11u64, 10u64), 8)).unwrap();
        for frame in 0..60u64 {
            let offset = Fraction::new(11u64, 10u64) + Fraction::new(8 * frame, 590u64);
            let resolved = resolve_layout(&layout(source.clone(), offset, 8)).unwrap();
            assert!(
                Arc::ptr_eq(&original, &resolved),
                "subcell animation rebuilt a snapped assembly"
            );
        }
        let resized =
            resolve_layout(&layout(source.clone(), Fraction::new(11u64, 10u64), 9)).unwrap();
        assert_eq!(original.source, resized.source);
        assert_ne!(
            original.placements, resized.placements,
            "inspector metadata must follow nonleaf layout changes"
        );
        let moved = resolve_layout(&layout(source.clone(), Fraction::from(2), 8)).unwrap();
        assert_ne!(original.source, moved.source);
        let edited = Arc::new(source.fill_color([0; 3], [1; 3], 0xff0000ff).unwrap());
        let edited = resolve_layout(&layout(edited, Fraction::new(11u64, 10u64), 8)).unwrap();
        assert_ne!(original.source, edited.source);
    }

    #[test]
    fn evicted_assembly_alias_replans_without_retaining_source_or_stale_output() {
        let source = Arc::new(sanim_voxel::Volume::new([4; 3]).unwrap());
        let node = layout(source, Fraction::from(1), 8);
        let original = resolve_layout(&node).unwrap();
        let recipe = LayoutKey::Recipe(node.fingerprint(), LayoutMode::Flattened);
        let assembly = LayoutKey::Assembly(
            sanim_voxel::layout::prepare(&node).unwrap().fingerprint(),
            LayoutMode::Flattened,
        );
        {
            let mut cache = layout_cache().lock().unwrap();
            cache.remove(&assembly);
            // Model the valid state where the alias outlives its LRU target.
            cache.insert(recipe, CachedLayout::RecipeAlias(assembly), 256);
        }
        let restored = resolve_layout(&node).unwrap();
        assert_eq!(original.source, restored.source);
        assert_eq!(original.placements, restored.placements);
        assert!(!Arc::ptr_eq(&original, &restored));
    }

    #[test]
    fn uniform_assemblies_cannot_hide_map_memory_from_the_cache_budget() {
        // Uniform payloads are tiny, but their many map entries still own memory.
        let source = Arc::new(
            sanim_voxel::Volume::new([512, 128, 128])
                .unwrap()
                .fill_color([0; 3], [512, 128, 128], 0x33aa55ff)
                .unwrap(),
        );
        assert!(source.resident_bytes() < MAX_LAYOUT_CACHE_BYTES);
        assert!(source.storage_metadata_bytes_since(None) > MAX_LAYOUT_CACHE_BYTES);
        let node = Arc::new(Node::part(source, Placement::default()).unwrap());
        let resolved = resolve_layout(&node).unwrap();
        let retained = Arc::downgrade(&resolved);
        drop(resolved);
        assert!(
            retained.upgrade().is_none(),
            "oversized map metadata must bypass the layout cache"
        );
    }
}
