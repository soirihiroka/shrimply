use sanim_voxel::{CHUNK_EDGE, Material, Texture, Volume, math::raycast};
use std::sync::Arc;

const BLUE: u32 = 0x3584e4ff;
const RED: u32 = 0xe01b24ff;

#[test]
fn sparse_edits_share_untouched_chunks_and_drop_emptied_chunks() {
    let volume = Volume::new([1024; 3]).unwrap();
    assert_eq!(volume.resident_bytes(), 0);
    let volume = volume.fill_color([0; 3], [16, 8, 8], BLUE).unwrap();
    let old_chunks = volume
        .chunks()
        .map(|(key, chunk)| (key, chunk.clone()))
        .collect::<Vec<_>>();
    assert_eq!(old_chunks.len(), 2);
    assert!(Arc::ptr_eq(&old_chunks[0].1, &old_chunks[1].1));
    let edited = volume.set([0, 0, 0], 0).unwrap();
    let new_chunks = edited
        .chunks()
        .map(|(key, chunk)| (key, chunk.clone()))
        .collect::<Vec<_>>();
    assert!(!Arc::ptr_eq(&old_chunks[0].1, &new_chunks[0].1));
    assert!(Arc::ptr_eq(&old_chunks[1].1, &new_chunks[1].1));
    assert_ne!(edited.get([0; 3]).unwrap(), volume.get([0; 3]).unwrap());
    let mut changes = Vec::new();
    assert_eq!(
        edited
            .visit_changes(&volume, |key, chunk| changes.push((key, chunk.is_some())))
            .unwrap(),
        1
    );
    assert_eq!(changes, [([0; 3], true)]);
    let cleared = edited.fill_box([0; 3], [8; 3], 0).unwrap();
    assert_eq!(cleared.chunk_count(), 1);
    assert_eq!(cleared.occupied_voxels(), CHUNK_EDGE.pow(3) as usize);
}

#[test]
fn unchanged_edits_preserve_snapshot_and_material_identities() {
    let volume = Volume::new([8; 3])
        .unwrap()
        .fill_color([0; 3], [8; 3], BLUE)
        .unwrap();
    let unchanged = volume.fill_color([0; 3], [8; 3], BLUE).unwrap();
    assert!(volume.same_storage(&unchanged));
    assert_eq!(volume.revision(), unchanged.revision());
    let (next, id) = volume
        .with_material(Arc::new(Material::solid(BLUE).unwrap()))
        .unwrap();
    assert_eq!(id, 1);
    assert!(volume.same_storage(&next));
}

#[test]
fn bounds_and_budgets_fail_without_modifying_the_original() {
    assert!(Volume::new([0, 8, 8]).is_err());
    let volume = Volume::new([8; 3]).unwrap();
    assert!(volume.set([8, 0, 0], 0).is_err());
    assert!(volume.fill_box([0; 3], [9, 8, 8], 0).is_err());
    assert!(
        volume
            .fill_box([0; 3], [8; 3], 1)
            .unwrap_err()
            .contains("material 1")
    );
    assert!(
        Material::solid(0xffffffff - 1)
            .unwrap_err()
            .contains("opaque")
    );
    let limits = sanim_voxel::Limits {
        max_chunks: 1,
        ..Default::default()
    };
    let volume = Volume::with_limits([16; 3], limits).unwrap();
    assert!(
        volume
            .fill_color([0; 3], [16; 3], BLUE)
            .unwrap_err()
            .contains("chunks")
    );
    assert_eq!(volume.chunk_count(), 0);
    assert_eq!(volume.materials().count(), 0);
}

#[test]
fn partial_boundary_chunks_never_count_padding_as_occupied() {
    let volume = Volume::new([9, 3, 2])
        .unwrap()
        .fill_color([0; 3], [9, 3, 2], RED)
        .unwrap();
    assert_eq!(volume.chunk_count(), 2);
    assert_eq!(volume.occupied_voxels(), 54);
    assert!(volume.chunks().all(|(_, chunk)| chunk.uniform().is_none()));
    let encoded = serde_json::to_string(&volume).unwrap();
    let restored: Volume = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.occupied_voxels(), 54);
    assert_eq!(restored.get([8, 2, 1]).unwrap(), 1);
}

#[test]
fn textures_are_shared_across_faces_materials_and_serialization() {
    let pixels: Arc<[u8]> = [255, 255, 255, 255, 0, 0, 0, 0].into();
    let texture = Arc::new(Texture::new(2, 1, pixels).unwrap());
    let faces = std::array::from_fn(|_| Some(texture.clone()));
    let material = Arc::new(Material::textured(BLUE, faces).unwrap());
    let volume = Volume::new([8; 3]).unwrap();
    let (volume, first) = volume.with_material(material.clone()).unwrap();
    let (same, second) = volume.with_material(material).unwrap();
    assert_eq!(first, second);
    assert!(volume.same_storage(&same));
    let volume = volume.fill_box([0; 3], [8; 3], first).unwrap();
    let restored: Volume = serde_json::from_str(&serde_json::to_string(&volume).unwrap()).unwrap();
    let faces = restored.material(1).unwrap().faces();
    assert!(Arc::ptr_eq(
        faces[0].as_ref().unwrap(),
        faces[5].as_ref().unwrap()
    ));
    assert_eq!(restored.resident_bytes(), volume.resident_bytes());
    assert!(
        Texture::new(1, 1, [0, 0, 0, 128].into())
            .unwrap_err()
            .contains("cutout")
    );
    assert!(Texture::new(0, 1, Arc::from([])).is_err());
}

#[test]
fn ray_traversal_handles_faces_parallel_axes_inside_origins_and_corner_ties() {
    let volume = Volume::new([8; 3])
        .unwrap()
        .fill_color([2; 3], [3; 3], RED)
        .unwrap();
    let hit = raycast(&volume, [-1.0, 2.5, 2.5], [1.0, 0.0, 0.0])
        .unwrap()
        .unwrap();
    assert_eq!(hit.position, [2; 3]);
    assert_eq!(hit.normal, [-1.0, 0.0, 0.0]);
    assert_eq!(hit.distance, 3.0);
    assert_eq!(hit.color, RED);
    let hit = raycast(&volume, [8.0, 2.5, 2.5], [-1.0, 0.0, 0.0])
        .unwrap()
        .unwrap();
    assert_eq!(hit.normal, [1.0, 0.0, 0.0]);
    assert_eq!(hit.distance, 5.0);
    assert_eq!(
        raycast(&volume, [2.5; 3], [1.0; 3])
            .unwrap()
            .unwrap()
            .distance,
        0.0
    );
    assert!(
        raycast(&volume, [-1.0, 8.0, 2.5], [1.0, 0.0, 0.0])
            .unwrap()
            .is_none()
    );
    assert!(raycast(&volume, [-1.0; 3], [-1.0; 3]).unwrap().is_none());
    assert!(raycast(&volume, [0.0; 3], [0.0; 3]).is_err());
    let corner = Volume::new([8; 3])
        .unwrap()
        .fill_color([1, 0, 0], [2, 1, 1], BLUE)
        .unwrap();
    assert!(raycast(&corner, [0.0; 3], [1.0; 3]).unwrap().is_none());
}

#[test]
fn emission_is_validated_and_part_of_material_identity() {
    assert!(
        Material::solid(BLUE)
            .unwrap()
            .with_emission(f32::NAN)
            .is_err()
    );
    assert!(Material::solid(BLUE).unwrap().with_emission(-1.0).is_err());
    let volume = Volume::new([8; 3]).unwrap();
    let (volume, first) = volume
        .with_material(Arc::new(Material::solid(BLUE).unwrap()))
        .unwrap();
    let (volume, second) = volume
        .with_material(Arc::new(
            Material::solid(BLUE).unwrap().with_emission(4.0).unwrap(),
        ))
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(volume.material(second).unwrap().emission(), 4.0);
}

#[test]
fn placement_remaps_palettes_preserves_holes_and_shares_aligned_bricks() {
    let child = Volume::new([8; 3])
        .unwrap()
        .fill_color([0; 3], [8; 3], BLUE)
        .unwrap();
    let empty = Volume::new([24; 3]).unwrap();
    let placed = empty.place(&child, [8; 3]).unwrap();
    assert!(Arc::ptr_eq(
        child.chunks().next().unwrap().1,
        placed.chunks().next().unwrap().1
    ));
    assert_eq!(placed.get([8; 3]).unwrap(), 1);
    assert_eq!(placed.occupied_voxels(), 512);
    assert!(empty.place(&child, [-1, 0, 0]).is_err());
    assert!(empty.place(&child, [17, 0, 0]).is_err());
    let parent = empty.fill_color([0; 3], [24; 3], RED).unwrap();
    let child = child.set([0; 3], 0).unwrap();
    let placed = parent.place(&child, [3, 4, 5]).unwrap();
    let red = parent.get([3, 4, 5]).unwrap();
    assert_eq!(placed.get([3, 4, 5]).unwrap(), red);
    let blue = placed.get([4, 4, 5]).unwrap();
    assert_eq!(placed.material(blue).unwrap().color(), BLUE);
    assert_eq!(placed.occupied_voxels(), 24usize.pow(3));
    assert_eq!(parent.materials().count(), 1);
    assert_eq!(placed.materials().count(), 2);
}

#[test]
fn fill_work_accounts_for_constant_solid_bricks_and_boundary_cells() {
    let volume = Volume::new([512; 3]).unwrap();
    assert_eq!(volume.fill_work([0; 3], [512, 8, 512]).unwrap(), 4096);
    assert_eq!(volume.fill_work([1; 3], [2; 3]).unwrap(), 512);
    assert_eq!(volume.fill_work([0; 3], [0; 3]).unwrap(), 0);
    assert!(volume.fill_work([0; 3], [513, 1, 1]).is_err());
}

#[test]
fn cutout_faces_continue_to_the_next_voxel_and_uv_orientation_is_explicit() {
    let clear = Arc::new(Texture::new(1, 1, [0, 0, 0, 0].into()).unwrap());
    let faces = std::array::from_fn(|_| Some(clear.clone()));
    let (volume, transparent) = Volume::new([8; 3])
        .unwrap()
        .with_material(Arc::new(Material::textured(0xffffffff, faces).unwrap()))
        .unwrap();
    let volume = volume
        .fill_box([1; 3], [2; 3], transparent)
        .unwrap()
        .fill_color([2, 1, 1], [3, 2, 2], BLUE)
        .unwrap();
    let hit = raycast(&volume, [-1.0, 1.5, 1.5], [1.0, 0.0, 0.0])
        .unwrap()
        .unwrap();
    assert_eq!(hit.position, [2, 1, 1]);
    assert_eq!(hit.color, BLUE);
    assert_eq!(
        sanim_voxel::math::face_uv([1, 0, 0], [3.0, 0.25, 0.75]),
        (0, [0.25, 0.75])
    );
    assert_eq!(
        sanim_voxel::math::face_uv([0, 1, 0], [0.25, 3.0, 0.75]),
        (2, [0.25, 0.75])
    );
}

#[test]
fn invalid_serialized_payloads_fail_before_they_can_create_invalid_cells() {
    assert!(
        serde_json::from_str::<Texture>("[1,1,[0,0,0,255,255]]")
            .unwrap_err()
            .to_string()
            .contains("more pixels")
    );
    assert!(
        serde_json::from_str::<Texture>("[1,1,[0,0,0]]")
            .unwrap_err()
            .to_string()
            .contains("fewer pixels")
    );
    assert!(
        serde_json::from_str::<Texture>("[4294967295,4294967295,[]]")
            .unwrap_err()
            .to_string()
            .contains("dimensions")
    );
    let value = r#"{"dimensions":[8,8,8],"textures":[],"materials":[],"chunks":[[[0,0,0],1]]}"#;
    assert!(
        serde_json::from_str::<Volume>(value)
            .unwrap_err()
            .to_string()
            .contains("absent material")
    );
    let volume = Volume::new([8; 3]).unwrap();
    assert!(
        volume
            .fill_color([0; 3], [0; 3], BLUE)
            .unwrap()
            .same_storage(&volume)
    );
}
