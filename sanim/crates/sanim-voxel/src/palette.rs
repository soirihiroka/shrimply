//! One immutable interning/remapping implementation for grids and GPU instance scenes.
use crate::{
    EMPTY, ENTRY_BYTES, MAX_RESIDENT_BYTES, Material, MaterialId, Texture, Volume, material,
    revision,
};
use imbl::OrdMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct MaterialPalette {
    pub(crate) materials: imbl::Vector<Arc<Material>>,
    pub(crate) keys: OrdMap<material::Key, MaterialId>,
    pub(crate) textures: OrdMap<usize, Arc<Texture>>,
    pub(crate) bytes: usize,
    pub(crate) revision: u64,
}
impl Default for MaterialPalette {
    fn default() -> Self {
        Self {
            materials: imbl::Vector::new(),
            keys: OrdMap::new(),
            textures: OrdMap::new(),
            bytes: 0,
            revision: revision(),
        }
    }
}
impl MaterialPalette {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn resident_bytes(&self) -> usize {
        self.bytes
    }
    /// Structural entries and allocation headers only; shared material/texture
    /// payloads are accounted through the caller's existing weak resource guards.
    pub fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + 2 * std::mem::size_of::<usize>()
            + ENTRY_BYTES * (self.materials.len() + self.textures.len())
    }
    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Arc<Material>)> {
        self.materials
            .iter()
            .enumerate()
            .map(|(index, material)| ((index + 1) as MaterialId, material))
    }
    pub fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        id.checked_sub(1)
            .and_then(|index| self.materials.get(index as usize))
    }
    pub fn with_material(
        self: &Arc<Self>,
        material: Arc<Material>,
    ) -> Result<(Arc<Self>, MaterialId), String> {
        let key = material.key();
        if let Some(id) = self.keys.get(&key) {
            return Ok((self.clone(), *id));
        }
        if self.materials.len() >= material::MAX_MATERIALS {
            return Err(format!(
                "voxel palette exceeds {} materials",
                material::MAX_MATERIALS
            ));
        }
        let mut palette = (**self).clone();
        let id = (palette.materials.len() + 1) as MaterialId;
        palette.bytes += ENTRY_BYTES + std::mem::size_of::<Material>();
        for texture in material.faces().iter().flatten() {
            let identity = Arc::as_ptr(texture) as usize;
            if !palette.textures.contains_key(&identity) {
                palette.bytes +=
                    ENTRY_BYTES + std::mem::size_of::<Texture>() + texture.pixels().len();
                palette.textures.insert(identity, texture.clone());
            }
        }
        if palette.bytes > MAX_RESIDENT_BYTES {
            return Err(format!(
                "voxel palette exceeds the {MAX_RESIDENT_BYTES} byte resident budget"
            ));
        }
        palette.materials.push_back(material);
        palette.keys.insert(key, id);
        palette.revision = revision();
        Ok((Arc::new(palette), id))
    }
    /// Intern only IDs actually used by an immutable source asset. Its geometry
    /// stays untouched, and its intrinsic used-ID index is computed at most once.
    pub fn with_volume(
        self: &Arc<Self>,
        source: &Volume,
    ) -> Result<(Arc<Self>, Arc<[MaterialId]>), String> {
        if Arc::ptr_eq(self, source.palette()) {
            return Ok((self.clone(), self.material_remap(source)?));
        }
        let mut palette = self.clone();
        let mut remap = vec![EMPTY; source.palette().materials.len() + 1];
        for id in source.used_materials().iter().copied() {
            let (next, mapped) = palette.with_material(
                source
                    .material(id)
                    .expect("validated source palette")
                    .clone(),
            )?;
            palette = next;
            remap[id as usize] = mapped;
        }
        Ok((palette, remap.into()))
    }
    pub fn material_remap(&self, source: &Volume) -> Result<Arc<[MaterialId]>, String> {
        if std::ptr::eq(self, source.palette().as_ref()) {
            return Ok((0..=source.palette().materials.len() as MaterialId)
                .collect::<Vec<_>>()
                .into());
        }
        let mut remap = vec![EMPTY; source.palette().materials.len() + 1];
        for id in source.used_materials().iter().copied() {
            remap[id as usize] = *self.keys.get(&source.material(id).expect("validated source palette").key()).ok_or_else(||format!("instance material {id} is missing from the composed palette; intern the asset with with_volume first"))?;
        }
        Ok(remap.into())
    }
}
