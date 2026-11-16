use sanim_voxel::{
    Volume,
    procedural::{
        Axes, Binary, Field, Heightmap, MAX_FIELD_NODES, Noise, NoiseKind, SurfacePoints, Unary,
    },
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
fn planar_noise_is_continuous_seeded_and_independent_of_the_excluded_axis() {
    let point = [13.17, -7.31, 25.61];
    for kind in NoiseKind::ALL {
        for axes in [Axes::Xy, Axes::Xz, Axes::Yz] {
            let field = Field::noise(Noise {
                seed: 71,
                kind,
                axes,
                ..Default::default()
            })
            .unwrap();
            let value = field.sample(point).unwrap();
            assert!((-1.0..=1.0).contains(&value));
            let excluded = axes.mask().iter().position(|included| !included).unwrap();
            let mut changed = point;
            changed[excluded] = 1e100;
            assert_eq!(value, field.sample(changed).unwrap(), "{kind:?} {axes:?}");
            changed = point;
            let included = axes.mask().iter().position(|included| *included).unwrap();
            changed[included] += 1e-6;
            assert!((value - field.sample(changed).unwrap()).abs() < 1e-5);
            let roundtrip: Field =
                serde_json::from_slice(&serde_json::to_vec(&field).unwrap()).unwrap();
            assert_eq!(value, roundtrip.sample(point).unwrap());
            assert_eq!(field.fingerprint(), roundtrip.fingerprint());
        }
    }
    let options = Noise {
        seed: 71,
        ..Default::default()
    };
    let xy = Field::noise(Noise {
        axes: Axes::Xy,
        ..options
    })
    .unwrap();
    let xz = Field::noise(Noise {
        axes: Axes::Xz,
        ..options
    })
    .unwrap();
    assert_eq!(
        xy.sample([13.17, 25.61, 400.0]).unwrap(),
        xz.sample(point).unwrap()
    );
}

#[test]
fn plane_warps_preserve_unselected_coordinates_and_bound_native_work() {
    let options = Noise {
        seed: 7,
        axes: Axes::Xz,
        ..Default::default()
    };
    let vertical = Field::axis(1).unwrap();
    assert_eq!(vertical.warp(options, 12.0).unwrap(), vertical);
    let horizontal = Field::axis(0).unwrap().warp(options, 12.0).unwrap();
    assert_eq!(
        horizontal.sample([13.17, 0.0, 25.61]).unwrap(),
        horizontal.sample([13.17, 500.0, 25.61]).unwrap()
    );
    assert_ne!(horizontal.sample([13.17, 0.0, 25.61]).unwrap(), 13.17);
    for kind in [NoiseKind::Value, NoiseKind::Gradient, NoiseKind::Cellular] {
        let a = Field::noise(Noise {
            kind,
            octaves: 1,
            ..options
        })
        .unwrap();
        let b = Field::noise(Noise {
            kind,
            octaves: 8,
            ..options
        })
        .unwrap();
        assert_eq!(
            a.sample_work(),
            b.sample_work(),
            "single-octave kernel {kind:?}"
        );
        assert_eq!(
            a.sample([13.17, 0.0, 25.61]).unwrap(),
            b.sample([13.17, 0.0, 25.61]).unwrap()
        );
    }
    let plane = Field::noise(Noise {
        kind: NoiseKind::Cellular,
        ..options
    })
    .unwrap();
    let volume = Field::noise(Noise {
        kind: NoiseKind::Cellular,
        axes: Axes::Xyz,
        ..options
    })
    .unwrap();
    assert!(plane.sample_work() < volume.sample_work());
    assert!(serde_json::from_str::<Noise>(r#"{"axes":"yx"}"#).is_err());
}

#[test]
fn smooth_masks_have_saturated_endpoints_flat_tangents_and_finite_bounds() {
    let mask = Field::axis(0).unwrap().smoothstep(-2.0, 2.0).unwrap();
    for (x, expected) in [
        (-f64::MAX, 0.0),
        (-2.0, 0.0),
        (0.0, 0.5),
        (2.0, 1.0),
        (f64::MAX, 1.0),
    ] {
        assert_eq!(mask.sample([x, 0.0, 0.0]).unwrap(), expected);
    }
    let step = 1e-4;
    assert!(mask.sample([-2.0 + step, 0.0, 0.0]).unwrap() / step < step);
    assert!((1.0 - mask.sample([2.0 - step, 0.0, 0.0]).unwrap()) / step < step);
    for (min, max) in [
        (1.0, 1.0),
        (2.0, 1.0),
        (f64::NAN, 1.0),
        (-f64::MAX, f64::MAX),
    ] {
        assert!(
            Field::axis(0)
                .unwrap()
                .smoothstep(min, max)
                .unwrap_err()
                .contains("min < max")
        );
    }
    let decoded: Field = serde_json::from_slice(&serde_json::to_vec(&mask).unwrap()).unwrap();
    assert_eq!(mask, decoded);
}

#[test]
fn pattern_fields_have_euclidean_periods_and_finite_native_trigonometry() {
    let axis = Field::axis(0).unwrap();
    assert_eq!(
        axis.unary(Unary::Floor)
            .unwrap()
            .sample([-1.25, 0.0, 0.0])
            .unwrap(),
        -2.0
    );
    let fractional = axis.unary(Unary::Fract).unwrap();
    assert_eq!(fractional.sample([-1.25, 0.0, 0.0]).unwrap(), 0.75);
    assert!((0.0..1.0).contains(&fractional.sample([-f64::MIN_POSITIVE, 0.0, 0.0]).unwrap()));
    for divisor in [2.0, -2.0] {
        let modulo = axis
            .binary(Binary::Modulo, &Field::constant(divisor).unwrap())
            .unwrap();
        assert_eq!(modulo.sample([-5.0, 0.0, 0.0]).unwrap(), 1.0);
        assert_eq!(modulo.sample([5.0, 0.0, 0.0]).unwrap(), 1.0);
    }
    assert!(
        axis.binary(Binary::Modulo, &Field::constant(0.0).unwrap())
            .unwrap()
            .sample([1.0, 0.0, 0.0])
            .unwrap_err()
            .contains("modulo divisor")
    );
    assert!(
        (axis
            .unary(Unary::Sin)
            .unwrap()
            .sample([std::f64::consts::FRAC_PI_2, 0.0, 0.0])
            .unwrap()
            - 1.0)
            .abs()
            < 1e-12
    );
    assert_eq!(
        axis.unary(Unary::Cos).unwrap().sample([0.0; 3]).unwrap(),
        1.0
    );
}

#[test]
fn surface_sampling_is_seeded_unique_and_filters_actual_raster_slopes() {
    use std::collections::BTreeSet;
    let ground = Volume::new([17, 20, 19])
        .unwrap()
        .fill_color([0; 3], [17, 8, 19], 0x55aa33ff)
        .unwrap();
    let options = SurfacePoints {
        step: 4,
        seed: 71,
        jitter: 1.0,
        ..Default::default()
    };
    let points = ground.surface_points(&options).unwrap();
    assert_eq!(points.len(), 25);
    assert_eq!(points.iter().copied().collect::<BTreeSet<_>>().len(), 25);
    assert!(points.iter().all(|[x, y, z]| *x < 17 && *z < 19 && *y == 8));
    assert_eq!(points, ground.surface_points(&options).unwrap());
    assert_ne!(
        points,
        ground
            .surface_points(&SurfacePoints {
                seed: 72,
                ..options
            })
            .unwrap()
    );
    let peak = ground
        .fill_color([8, 8, 8], [9, 14, 9], 0x888888ff)
        .unwrap();
    let flat = peak
        .surface_points(&SurfacePoints {
            step: 1,
            max_slope: Some(0.0),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        flat.len(),
        17 * 19 - 5,
        "the peak and all four cliff-adjacent columns are rejected"
    );
    let mask = Field::axis(0)
        .unwrap()
        .binary(Binary::Subtract, &Field::constant(8.0).unwrap())
        .unwrap();
    let selected = peak
        .surface_points(&SurfacePoints {
            step: 1,
            mask: Some(&mask),
            ..Default::default()
        })
        .unwrap();
    assert!(selected.iter().all(|point| point[0] >= 8));
    let zero = Field::constant(0.0).unwrap();
    assert!(
        ground
            .surface_points(&SurfacePoints {
                density: Some(&zero),
                ..options
            })
            .unwrap()
            .is_empty()
    );
    let density = Field::constant(0.4).unwrap();
    assert!(
        ground
            .surface_points_work(&SurfacePoints {
                density: Some(&density),
                mask: Some(&mask),
                max_slope: Some(1.0),
                ..options
            })
            .unwrap()
            > ground.surface_points_work(&options).unwrap()
    );
    for invalid in [
        SurfacePoints {
            jitter: 1.1,
            ..options
        },
        SurfacePoints {
            jitter: f64::NAN,
            ..options
        },
        SurfacePoints {
            max_slope: Some(-1.0),
            ..options
        },
        SurfacePoints { step: 0, ..options },
    ] {
        assert!(ground.surface_points_work(&invalid).is_err());
    }
    let large = Volume::new([257, 3, 257]).unwrap();
    assert!(
        large
            .surface_points_work(&SurfacePoints {
                step: 1,
                ..Default::default()
            })
            .unwrap_err()
            .contains("points")
    );
    assert!(
        large.scatter_work(&density, 1).is_ok(),
        "dense native scatter does not allocate a List or inherit its point count cap"
    );
}

#[test]
fn density_carving_reuses_chunk_rasterization_and_preserves_old_snapshots() {
    let ground = Volume::new([24; 3])
        .unwrap()
        .fill_color([0; 3], [24; 3], 0x888888ff)
        .unwrap();
    let sphere = Field::constant(5.0)
        .unwrap()
        .binary(
            Binary::Subtract,
            &Field::distance([12.0; 3], [true; 3]).unwrap(),
        )
        .unwrap();
    let carved = ground
        .fill_density(&sphere, [4; 3], [20; 3], 0.0, sanim_voxel::EMPTY)
        .unwrap();
    assert_eq!(carved.get([12; 3]).unwrap(), sanim_voxel::EMPTY);
    assert_ne!(ground.get([12; 3]).unwrap(), sanim_voxel::EMPTY);
    assert_eq!(ground.get([0; 3]).unwrap(), carved.get([0; 3]).unwrap());
    let old_chunk = ground.chunks().find(|(at, _)| *at == [0; 3]).unwrap().1;
    let unchanged = carved.chunks().find(|(at, _)| *at == [0; 3]).unwrap().1;
    assert!(Arc::ptr_eq(old_chunk, unchanged));
    let rejected = ground
        .fill_density(
            &Field::constant(-1.0).unwrap(),
            [0; 3],
            [24; 3],
            0.0,
            sanim_voxel::EMPTY,
        )
        .unwrap();
    assert!(ground.same_storage(&rejected));
}

#[test]
fn foliage_cards_do_not_become_support_for_later_scatter_layers() {
    let ground = Volume::new([8, 16, 8])
        .unwrap()
        .fill_color([0; 3], [8, 8, 8], 0x55aa33ff)
        .unwrap();
    let card = Arc::new(
        sanim_voxel::Material::solid(0x338833ff)
            .unwrap()
            .with_geometry(sanim_voxel::MaterialGeometry::Cross)
            .unwrap(),
    );
    let (ground, grass) = ground.with_material(card).unwrap();
    let (ground, flower) = ground
        .with_material(Arc::new(
            sanim_voxel::Material::solid(0xffcc33ff)
                .unwrap()
                .with_geometry(sanim_voxel::MaterialGeometry::Cross)
                .unwrap(),
        ))
        .unwrap();
    let one = Field::constant(1.0).unwrap();
    let plants = ground.scatter(&one, grass, 1, 1).unwrap();
    let flowers = plants.scatter(&one, flower, 2, 1).unwrap();
    assert_eq!(flowers.get([4, 8, 4]).unwrap(), flower);
    assert_eq!(flowers.get([4, 9, 4]).unwrap(), sanim_voxel::EMPTY);
    assert_eq!(plants.get([4, 8, 4]).unwrap(), grass);
    assert!(
        flowers
            .surface_points(&SurfacePoints::default())
            .unwrap()
            .iter()
            .all(|point| point[1] == 8)
    );
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
    let points = painted
        .surface_points(&SurfacePoints {
            step: 4,
            ..Default::default()
        })
        .unwrap();
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
    let mut medians = Vec::new();
    for axes in [Axes::Xyz, Axes::Xz] {
        let field = Field::noise(Noise {
            seed: 42,
            axes,
            ..Default::default()
        })
        .unwrap();
        let mut sampler = field.sampler();
        let mut times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let mut total = 0.0;
            for z in 0..128 {
                for x in 0..128 {
                    total += sampler
                        .sample([f64::from(x) + 0.5, 0.0, f64::from(z) + 0.5])
                        .unwrap();
                }
            }
            std::hint::black_box(total);
            times.push(start.elapsed());
        }
        times.sort_unstable();
        medians.push(times[2]);
        eprintln!(
            "noise axes={} four_octave_samples=16384 median_us={} retained_bytes={}",
            axes.name(),
            times[2].as_micros(),
            field.retained_bytes()
        );
    }
    assert!(
        medians[1] < medians[0],
        "plane-specific kernels must reduce native sampling cost"
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
