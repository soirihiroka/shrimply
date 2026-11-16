use sanim_voxel::{
    Volume,
    procedural::{Binary, Field, Heightmap, MAX_FIELD_NODES, Noise, NoiseKind, Unary},
};
use std::sync::Arc;

#[test]
fn field_program_shares_subexpressions_and_survives_serialization() {
    let mut field = Field::axis(0).unwrap();
    for _ in 0..20 {
        field = field.binary(Binary::Add, &field).unwrap();
    }
    assert_eq!(
        field.instructions(),
        21,
        "doubling a DAG must add one instruction, not duplicate its history"
    );
    assert_eq!(
        field.sample([3.0, 0.0, 0.0]).unwrap(),
        3.0 * (1u64 << 20) as f64
    );
    let encoded = serde_json::to_vec(&field).unwrap();
    let decoded: Field = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(field, decoded);
    assert_eq!(field.fingerprint(), decoded.fingerprint());
    for _ in 21..MAX_FIELD_NODES {
        field = field.unary(Unary::Negate).unwrap();
    }
    assert!(
        field
            .unary(Unary::Negate)
            .unwrap_err()
            .contains("instruction")
    );
    assert!(
        serde_json::from_str::<Field>(r#"[{"Binary":["Add",1,1]}]"#)
            .unwrap_err()
            .to_string()
            .contains("earlier")
    );
}

#[test]
fn noise_is_seeded_bounded_continuous_and_order_independent() {
    for kind in NoiseKind::ALL {
        let field = Field::noise(Noise {
            seed: 73,
            kind,
            ..Default::default()
        })
        .unwrap();
        let other = Field::noise(Noise {
            seed: 74,
            kind,
            ..Default::default()
        })
        .unwrap();
        let point = [3.13, 7.29, -19.41];
        let value = field.sample(point).unwrap();
        assert!((-1.0..=1.0).contains(&value));
        assert_ne!(value, other.sample(point).unwrap());
        field.sample([400.0, -20.0, 1.0]).unwrap();
        assert_eq!(value, field.sample(point).unwrap());
        assert!(
            (value
                - field
                    .sample([point[0] + 0.00001, point[1], point[2]])
                    .unwrap())
            .abs()
                < 0.001
        );
    }
    let value = sanim_voxel::procedural::random(42, 17);
    assert!((0.0..1.0).contains(&value));
    assert_eq!(value, sanim_voxel::procedural::random(42, 17));
    assert_ne!(value, sanim_voxel::procedural::random(42, 18));
}

#[test]
fn transforms_warp_and_runtime_domain_errors_are_explicit() {
    let field = Field::axis(0)
        .unwrap()
        .transform([4.0, 0.0, 0.0], [2.0; 3])
        .unwrap();
    assert_eq!(field.sample([10.0, 0.0, 0.0]).unwrap(), 3.0);
    assert!(
        field.instructions() <= 5,
        "unused domain axes must be removed"
    );
    let radial = Field::distance([4.0, 100.0, 3.0], [true, false, true]).unwrap();
    assert_eq!(radial.sample([7.0, -100.0, 7.0]).unwrap(), 5.0);
    assert_eq!(radial.warp(Noise::default(), 0.0).unwrap(), radial);
    let warped = radial
        .warp(
            Noise {
                seed: 9,
                ..Default::default()
            },
            2.0,
        )
        .unwrap();
    assert_ne!(warped.sample([7.0, 0.0, 7.0]).unwrap(), 5.0);
    let bad = field
        .binary(Binary::Divide, &Field::constant(0.0).unwrap())
        .unwrap();
    let error = bad.sample([1.0, 2.0, 3.0]).unwrap_err();
    assert!(error.contains("non-finite") && error.contains("[1.0, 2.0, 3.0]"));
}

#[test]
fn heightmap_fills_exact_columns_and_reuses_uniform_bricks() {
    let original = Volume::new([32, 24, 32]).unwrap();
    let (base, id) = original
        .with_material(Arc::new(sanim_voxel::Material::solid(0x55aa33ff).unwrap()))
        .unwrap();
    let field = Field::constant(17.9).unwrap();
    let plan = Heightmap::new(&field, base.dimensions()).unwrap();
    let filled = plan.apply(&base, id).unwrap();
    assert_eq!(filled.occupied_voxels(), 32 * 17 * 32);
    assert_eq!(original.occupied_voxels(), 0);
    assert_eq!(filled.get([16, 16, 16]).unwrap(), id);
    assert_eq!(filled.get([16, 17, 16]).unwrap(), 0);
    let full = filled
        .chunks()
        .filter(|(_, chunk)| chunk.uniform() == Some(id))
        .map(|(_, chunk)| chunk.clone())
        .collect::<Vec<_>>();
    assert!(full.len() > 2);
    assert!(full.iter().all(|chunk| Arc::ptr_eq(chunk, &full[0])));
    assert!(Heightmap::new(&field, [u32::MAX; 3]).is_err());
}

#[test]
fn density_masks_and_surface_paint_preserve_cavities_and_snapshots() {
    let (base, stone) = Volume::new([16; 3])
        .unwrap()
        .with_material(Arc::new(sanim_voxel::Material::solid(0x888888ff).unwrap()))
        .unwrap();
    let sphere = Field::constant(6.0)
        .unwrap()
        .binary(
            Binary::Subtract,
            &Field::distance([8.0; 3], [true; 3]).unwrap(),
        )
        .unwrap();
    let filled = base
        .fill_density(&sphere, [0; 3], [16; 3], 0.0, stone)
        .unwrap();
    assert_eq!(filled.get([0; 3]).unwrap(), 0);
    assert_eq!(filled.get([8; 3]).unwrap(), stone);
    let hollow = filled.fill_box([7, 10, 7], [9, 12, 9], 0).unwrap();
    let (hollow, grass) = hollow
        .with_material(Arc::new(sanim_voxel::Material::solid(0x55aa33ff).unwrap()))
        .unwrap();
    let painted = hollow.paint_surface(grass, 4, None).unwrap();
    assert_eq!(
        painted.get([8, 10, 8]).unwrap(),
        0,
        "surface paint must not fill an internal cavity"
    );
    assert_eq!(painted.get([8, 13, 8]).unwrap(), grass);
    assert_eq!(hollow.get([8, 13, 8]).unwrap(), stone);
    let points = painted.surface_points(4).unwrap();
    for [x, y, z] in points {
        assert_ne!(painted.get([x, y - 1, z]).unwrap(), 0);
        assert_eq!(painted.get([x, y, z]).unwrap(), 0);
    }
}

#[test]
fn batch_palette_mapping_preserves_shared_geometry_for_gpu_instances() {
    let asset = Volume::new([8; 3])
        .unwrap()
        .fill_color([0; 3], [8, 4, 8], 0x33aaffff)
        .unwrap()
        .fill_color([0, 4, 0], [4, 8, 8], 0xff7733ff)
        .unwrap();
    let parent = Volume::new([32, 8, 8])
        .unwrap()
        .fill_color([8, 0, 0], [16, 8, 8], 0x55aa33ff)
        .unwrap();
    let batch = parent.place_many(&asset, &[[0, 0, 0], [16, 0, 0]]).unwrap();
    let repeated = parent
        .place(&asset, [0, 0, 0])
        .unwrap()
        .place(&asset, [16, 0, 0])
        .unwrap();
    assert_eq!(batch, repeated);
    let a = batch.chunks().find(|(key, _)| *key == [0, 0, 0]).unwrap().1;
    let b = batch.chunks().find(|(key, _)| *key == [2, 0, 0]).unwrap().1;
    assert!(
        Arc::ptr_eq(a, b),
        "palette remapping must happen once per shared asset, not separately per placement"
    );
    assert!(parent.same_storage(&parent.place_many(&asset, &[]).unwrap()));
    assert!(parent.place_many(&asset, &[[31, 0, 0]]).is_err());
}

#[test]
#[ignore = "manual native field/chunk batching benchmark"]
fn procedural_raster_benchmark() {
    use std::time::Instant;
    let field = Field::noise(Noise {
        seed: 42,
        scale: 24.0,
        ..Default::default()
    })
    .unwrap()
    .binary(Binary::Multiply, &Field::constant(8.0).unwrap())
    .unwrap()
    .binary(Binary::Add, &Field::constant(16.0).unwrap())
    .unwrap();
    let (base, id) = Volume::new([96, 40, 96])
        .unwrap()
        .with_material(Arc::new(sanim_voxel::Material::solid(0x55aa33ff).unwrap()))
        .unwrap();
    let mut native = Vec::new();
    let mut scalar = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let plan = Heightmap::new(&field, base.dimensions()).unwrap();
        let fast = plan.apply(&base, id).unwrap();
        native.push(started.elapsed());
        let started = Instant::now();
        let mut slow = base.clone();
        for z in 0..96 {
            for x in 0..96 {
                let height = field
                    .sample([f64::from(x) + 0.5, 0.0, f64::from(z) + 0.5])
                    .unwrap()
                    .floor()
                    .clamp(0.0, 40.0) as u32;
                slow = slow
                    .fill_box([x, 0, z], [x + 1, height, z + 1], id)
                    .unwrap();
            }
        }
        scalar.push(started.elapsed());
        assert_eq!(fast, slow);
    }
    native.sort();
    scalar.sort();
    eprintln!(
        "native_heightmap_us={} repeated_column_edits_us={} ratio={:.2}",
        native[1].as_micros(),
        scalar[1].as_micros(),
        scalar[1].as_secs_f64() / native[1].as_secs_f64()
    );
    assert!(
        native[1] < scalar[1],
        "native chunk batching must earn its complexity on representative terrain"
    );
}

#[test]
fn material_indices_are_cached_on_immutable_storage_with_prepaid_memory() {
    let source = Volume::new([16; 3])
        .unwrap()
        .fill_color([0; 3], [8; 3], 0x33aaffff)
        .unwrap();
    let before = source.storage_metadata_bytes_since(None);
    let indices = source.used_materials().clone();
    assert_eq!(indices.as_ref(), &[1]);
    assert!(Arc::ptr_eq(&indices, source.used_materials()));
    assert_eq!(
        before,
        source.storage_metadata_bytes_since(None),
        "warming intrinsic metadata must not invalidate its earlier memory proof"
    );
    let edited = source.fill_color([0; 3], [1; 3], 0xff7733ff).unwrap();
    assert_eq!(edited.used_materials().as_ref(), &[1, 2]);
    assert_eq!(source.used_materials().as_ref(), &[1]);
    let world = Volume::new([32; 3])
        .unwrap()
        .place(&source, [0; 3])
        .unwrap();
    assert_eq!(world.material_remap(&source).unwrap().as_ref(), &[0, 1]);
}

#[test]
fn surface_mask_selects_columns_at_their_top_without_filling_cavities() {
    let source = Volume::new([16, 16, 16])
        .unwrap()
        .fill_color([0, 0, 0], [16, 8, 16], 0x888888ff)
        .unwrap();
    let (source, grass) = source
        .with_material(Arc::new(sanim_voxel::Material::solid(0x33aa55ff).unwrap()))
        .unwrap();
    let mask = Field::axis(0)
        .unwrap()
        .binary(Binary::Subtract, &Field::constant(8.0).unwrap())
        .unwrap();
    let painted = source.paint_surface(grass, 2, Some(&mask)).unwrap();
    for x in 0..16 {
        for y in 0..16 {
            assert_eq!(
                painted.get([x, y, 0]).unwrap(),
                if y >= 8 {
                    0
                } else if y >= 6 && x >= 8 {
                    grass
                } else {
                    1
                }
            );
        }
    }
    let rejected = source
        .paint_surface(grass, 2, Some(&Field::constant(0.0).unwrap()))
        .unwrap();
    assert!(source.same_storage(&rejected));
    assert_eq!(source.get([12, 7, 0]).unwrap(), 1);
}

#[test]
fn flood_fills_only_air_and_keeps_waterline_and_source_exact() {
    let ground = Volume::new([17, 16, 17])
        .unwrap()
        .fill_color([0, 0, 0], [17, 4, 17], 0x55aa33ff)
        .unwrap();
    let (ground, water) = ground
        .with_material(Arc::new(sanim_voxel::Material::solid(0x3584e4ff).unwrap()))
        .unwrap();
    let flooded = ground.flood(9, water).unwrap();
    for z in 0..17 {
        for y in 0..16 {
            for x in 0..17 {
                assert_eq!(
                    flooded.get([x, y, z]).unwrap(),
                    if y < 4 {
                        1
                    } else if y < 9 {
                        water
                    } else {
                        0
                    }
                );
            }
        }
    }
    assert!(ground.same_storage(&ground.flood(0, water).unwrap()));
    assert_eq!(ground.get([8, 4, 8]).unwrap(), 0);
    assert!(ground.flood(17, water).unwrap_err().contains("height"));
}

#[test]
fn foliage_scatter_is_seeded_surface_bound_and_preserves_unselected_columns() {
    let ground = Volume::new([16, 12, 16])
        .unwrap()
        .fill_color([0; 3], [16, 8, 16], 0x55aa33ff)
        .unwrap();
    let (ground, plant) = ground
        .with_material(Arc::new(sanim_voxel::Material::solid(0x33aa55ff).unwrap()))
        .unwrap();
    let density = Field::constant(0.4).unwrap();
    let a = ground.scatter(&density, plant, 73, 1).unwrap();
    let b = ground.scatter(&density, plant, 73, 1).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, ground.scatter(&density, plant, 74, 1).unwrap());
    let mut count = 0;
    for z in 0..16 {
        for x in 0..16 {
            let value = a.get([x, 8, z]).unwrap();
            if value != 0 {
                assert_eq!(value, plant);
                count += 1;
            }
            assert_eq!(a.get([x, 9, z]).unwrap(), 0);
            assert_eq!(a.get([x, 7, z]).unwrap(), ground.get([x, 7, z]).unwrap());
        }
    }
    assert!((60..150).contains(&count));
    assert!(
        ground.same_storage(
            &ground
                .scatter(&Field::constant(0.0).unwrap(), plant, 73, 1)
                .unwrap()
        )
    );
    assert!(ground.scatter(&density, plant, 73, 0).is_err());
}
