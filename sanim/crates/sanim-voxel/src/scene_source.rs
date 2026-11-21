//! One occupancy interface keeps CPU traversal identical for flattened and instanced scenes.
use crate::{InstanceScene, Material, MaterialId, MaterialPalette, Volume};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
/// Read-only occupancy shared by sparse volumes and instanced assets.
pub trait GridView {
    fn dimensions(&self) -> [u32; 3];
    fn cell(&self, position: [u32; 3]) -> MaterialId;
    fn material(&self, id: MaterialId) -> Option<&Arc<Material>>;
}
impl GridView for Volume {
    fn dimensions(&self) -> [u32; 3] {
        Volume::dimensions(self)
    }
    fn cell(&self, position: [u32; 3]) -> MaterialId {
        Volume::cell(self, position)
    }
    fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        Volume::material(self, id)
    }
}
impl<T: GridView + ?Sized> GridView for Arc<T> {
    fn dimensions(&self) -> [u32; 3] {
        self.as_ref().dimensions()
    }
    fn cell(&self, position: [u32; 3]) -> MaterialId {
        self.as_ref().cell(position)
    }
    fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        self.as_ref().material(id)
    }
}
impl GridView for InstanceScene {
    fn dimensions(&self) -> [u32; 3] {
        InstanceScene::dimensions(self)
    }
    fn cell(&self, position: [u32; 3]) -> MaterialId {
        InstanceScene::cell(self, position)
    }
    fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        InstanceScene::material(self, id)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneSource {
    Volume(Arc<Volume>),
    Instances(Arc<InstanceScene>),
}
impl SceneSource {
    pub fn dimensions(&self) -> [u32; 3] {
        match self {
            Self::Volume(volume) => volume.dimensions(),
            Self::Instances(scene) => scene.dimensions(),
        }
    }
    pub fn palette(&self) -> &Arc<MaterialPalette> {
        match self {
            Self::Volume(volume) => volume.palette(),
            Self::Instances(scene) => scene.palette(),
        }
    }
    pub fn materials(&self) -> impl Iterator<Item = (MaterialId, &Arc<Material>)> {
        self.palette().materials()
    }
    pub fn material(&self, id: MaterialId) -> Option<&Arc<Material>> {
        self.palette().material(id)
    }
    pub fn revision(&self) -> u64 {
        match self {
            Self::Volume(volume) => volume.revision(),
            Self::Instances(scene) => scene.revision(),
        }
    }
    pub fn resident_bytes(&self) -> usize {
        match self {
            Self::Volume(volume) => {
                volume.resident_bytes()
                    + volume.storage_metadata_bytes_since(None)
                    + std::mem::size_of::<Volume>()
                    + std::mem::size_of::<MaterialPalette>()
                    + 4 * std::mem::size_of::<usize>()
            }
            Self::Instances(scene) => scene.resident_bytes(),
        }
    }
    pub fn occupied_voxels(&self) -> Option<usize> {
        match self {
            Self::Volume(volume) => Some(volume.occupied_voxels()),
            Self::Instances(_) => None,
        }
    }
    pub fn occupied_voxels_upper_bound(&self) -> usize {
        match self {
            Self::Volume(volume) => volume.occupied_voxels(),
            Self::Instances(scene) => scene.occupied_voxels_upper_bound(),
        }
    }
    pub fn flatten(&self) -> Result<Arc<Volume>, String> {
        match self {
            Self::Volume(volume) => Ok(volume.clone()),
            Self::Instances(scene) => scene.flatten(),
        }
    }
    pub fn get(&self, position: [u32; 3]) -> Result<MaterialId, String> {
        if (0..3).any(|axis| position[axis] >= self.dimensions()[axis]) {
            return Err(format!(
                "voxel position {position:?} is outside scene dimensions {:?}",
                self.dimensions()
            ));
        }
        Ok(match self {
            Self::Volume(volume) => volume.cell(position),
            Self::Instances(scene) => scene.cell(position),
        })
    }
    pub fn raycast(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
    ) -> Result<Option<crate::math::Hit>, String> {
        match self {
            Self::Volume(volume) => crate::math::raycast(volume.as_ref(), origin, direction),
            Self::Instances(scene) => crate::math::raycast(scene.as_ref(), origin, direction),
        }
    }
    pub fn same_snapshot(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Volume(a), Self::Volume(b)) => Arc::ptr_eq(a, b),
            (Self::Instances(a), Self::Instances(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
