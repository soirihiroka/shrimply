use sanim_voxel::{Material, MaterialGeometry, OpticalProperties, Texture, Volume, math::raycast};
use std::sync::Arc;

#[test]
fn opaque_defaults_keep_existing_wire_values_and_palette_identity() {
    let material = Material::solid(0x3584e4ff).unwrap();
    assert_eq!(*material.optics(), OpticalProperties::default());
    assert_eq!(material.geometry(), MaterialGeometry::Cube);
    let encoded = serde_json::to_value(&material).unwrap();
    assert!(encoded.get("optics").is_none() && encoded.get("geometry").is_none());
    let restored: Material = serde_json::from_value(encoded).unwrap();
    assert_eq!(material, restored);
    let (volume, id) = Volume::new([8; 3])
        .unwrap()
        .with_material(Arc::new(material.clone()))
        .unwrap();
    let equivalent = material
        .with_optics(OpticalProperties {
            transmission: -0.,
            roughness: -0.,
            wave_strength: -0.,
            ..Default::default()
        })
        .unwrap();
    let (same, equivalent_id) = volume.with_material(Arc::new(equivalent)).unwrap();
    assert_eq!(id, equivalent_id);
    assert!(volume.same_storage(&same));
}

#[test]
fn every_optical_field_and_geometry_participates_in_palette_identity() {
    let original = Material::solid(0xffffffff).unwrap();
    let mut variants = Vec::new();
    for optics in [
        OpticalProperties {
            transmission: 1.,
            ..Default::default()
        },
        OpticalProperties {
            ior: 1.333,
            ..Default::default()
        },
        OpticalProperties {
            roughness: 0.1,
            ..Default::default()
        },
        OpticalProperties {
            attenuation_color: [0.5, 1., 1.],
            ..Default::default()
        },
        OpticalProperties {
            attenuation_color: [1., 0.5, 1.],
            ..Default::default()
        },
        OpticalProperties {
            attenuation_color: [1., 1., 0.5],
            ..Default::default()
        },
        OpticalProperties {
            attenuation_distance: 8.,
            ..Default::default()
        },
        OpticalProperties {
            wave_strength: 0.1,
            ..Default::default()
        },
        OpticalProperties {
            wave_length: 12.,
            ..Default::default()
        },
        OpticalProperties {
            wave_speed: 2.,
            ..Default::default()
        },
    ] {
        variants.push(original.clone().with_optics(optics).unwrap());
    }
    variants.push(
        original
            .clone()
            .with_geometry(MaterialGeometry::Cross)
            .unwrap(),
    );
    let (mut volume, first) = Volume::new([8; 3])
        .unwrap()
        .with_material(Arc::new(original))
        .unwrap();
    for (index, material) in variants.into_iter().enumerate() {
        let (next, id) = volume.with_material(Arc::new(material)).unwrap();
        assert_eq!(id, index as u32 + first + 1);
        volume = next;
    }
}

#[test]
fn volume_roundtrip_preserves_optics_cards_and_legacy_materials() {
    let water = Material::solid(0xffffffff)
        .unwrap()
        .with_optics(OpticalProperties {
            transmission: 1.,
            ior: 1.333,
            roughness: 0.03,
            attenuation_color: [0.3, 0.7, 0.8],
            attenuation_distance: 8.,
            wave_strength: 0.08,
            ..Default::default()
        })
        .unwrap();
    let (volume, id) = Volume::new([8; 3])
        .unwrap()
        .with_material(Arc::new(water))
        .unwrap();
    let volume = volume.fill_box([0; 3], [8; 3], id).unwrap();
    let encoded = serde_json::to_value(&volume).unwrap();
    let restored: Volume = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(restored, volume);
    assert_eq!(restored.resident_bytes(), volume.resident_bytes());
    let mut misspelled = encoded.clone();
    misspelled["materials"][0]["optics"]["transmisson"] = serde_json::json!(1.);
    assert!(
        serde_json::from_value::<Volume>(misspelled)
            .unwrap_err()
            .to_string()
            .contains("unknown field")
    );
    let mut invalid = encoded;
    invalid["materials"][0]["optics"]["ior"] = serde_json::json!(0.);
    assert!(
        serde_json::from_value::<Volume>(invalid)
            .unwrap_err()
            .to_string()
            .contains("ior")
    );
    let cross = Material::solid(0xffffffff)
        .unwrap()
        .with_geometry(MaterialGeometry::Cross)
        .unwrap();
    let restored: Material = serde_json::from_str(&serde_json::to_string(&cross).unwrap()).unwrap();
    assert_eq!(restored.geometry(), MaterialGeometry::Cross);
}

#[test]
fn numerical_ranges_and_closed_medium_constraints_fail_clearly() {
    for (optics, field) in [
        (
            OpticalProperties {
                transmission: 1.1,
                ..Default::default()
            },
            "transmission",
        ),
        (
            OpticalProperties {
                ior: f32::NAN,
                ..Default::default()
            },
            "ior",
        ),
        (
            OpticalProperties {
                roughness: -0.1,
                ..Default::default()
            },
            "roughness",
        ),
        (
            OpticalProperties {
                attenuation_distance: 0.,
                ..Default::default()
            },
            "attenuation_distance",
        ),
        (
            OpticalProperties {
                attenuation_color: [0.; 3],
                attenuation_distance: 1e-40,
                ..Default::default()
            },
            "GPU numeric range",
        ),
        (
            OpticalProperties {
                wave_strength: 2.,
                ..Default::default()
            },
            "wave_strength",
        ),
        (
            OpticalProperties {
                wave_length: 0.,
                ..Default::default()
            },
            "wave_length",
        ),
        (
            OpticalProperties {
                wave_speed: 65.,
                ..Default::default()
            },
            "wave_speed",
        ),
    ] {
        assert!(
            Material::solid(0xffffffff)
                .unwrap()
                .with_optics(optics)
                .unwrap_err()
                .contains(field)
        );
    }
    let glass = OpticalProperties {
        transmission: 1.,
        ..Default::default()
    };
    assert!(
        Material::solid(0xffffffff)
            .unwrap()
            .with_geometry(MaterialGeometry::Cross)
            .unwrap()
            .with_optics(glass)
            .unwrap_err()
            .contains("closed cube")
    );
    assert!(
        Material::solid(0xffffffff)
            .unwrap()
            .with_optics(glass)
            .unwrap()
            .with_geometry(MaterialGeometry::Cross)
            .unwrap_err()
            .contains("closed cube")
    );
    let cutout = Arc::new(Texture::new(1, 1, Arc::from([0, 0, 0, 0])).unwrap());
    let material =
        Material::textured(0xffffffff, std::array::from_fn(|_| Some(cutout.clone()))).unwrap();
    assert!(
        material
            .clone()
            .with_optics(glass)
            .unwrap_err()
            .contains("closed opaque")
    );
    material.with_geometry(MaterialGeometry::Cross).unwrap();
    let partial = Arc::new(Texture::new(1, 1, Arc::from([255, 255, 255, 128])).unwrap());
    assert!(
        Material::textured(0xffffffff, std::array::from_fn(|_| Some(partial.clone())))
            .unwrap_err()
            .contains("transmission")
    );
}

#[test]
fn beer_lambert_depends_on_distance_not_voxel_partition() {
    let optics = OpticalProperties {
        attenuation_color: [0.2, 0.5, 0.8],
        attenuation_distance: 4.,
        ..Default::default()
    };
    let all = optics.transmittance(12.).unwrap();
    let segment = optics.transmittance(3.).unwrap();
    for axis in 0..3 {
        assert!((all[axis] - segment[axis].powi(4)).abs() < 1e-12);
    }
    assert_eq!(optics.transmittance(0.).unwrap(), [1.; 3]);
    assert_eq!(OpticalProperties::default().absorption(), [0.; 3]);
    assert!(
        OpticalProperties {
            attenuation_color: [0.; 3],
            ..Default::default()
        }
        .absorption()
        .iter()
        .all(|value| value.is_finite())
    );
}

#[test]
fn inspection_picks_the_water_surface_instead_of_silently_skipping_it() {
    let (volume, water) = Volume::new([4; 3])
        .unwrap()
        .with_material(Arc::new(
            Material::solid(0xffffffff)
                .unwrap()
                .with_optics(OpticalProperties {
                    transmission: 1.,
                    ..Default::default()
                })
                .unwrap(),
        ))
        .unwrap();
    let volume = volume
        .fill_box([0, 0, 1], [4, 4, 3], water)
        .unwrap()
        .fill_color([0; 3], [4, 4, 1], 0xff0000ff)
        .unwrap();
    let hit = raycast(&volume, [1.5, 1.5, 8.], [0., 0., -1.])
        .unwrap()
        .unwrap();
    assert_eq!(hit.material, water);
    assert_eq!(hit.position, [1, 1, 2]);
}

#[test]
#[ignore = "manual opaque-default optical configuration cost measurement"]
fn opaque_optics_configuration_benchmark() {
    use std::{hint::black_box, time::Instant};
    let iterations = 200_000;
    let mut reports = Vec::new();
    for configured in [false, true] {
        let mut samples = Vec::new();
        for _ in 0..9 {
            let started = Instant::now();
            for _ in 0..iterations {
                let material = Material::solid(black_box(0xffffffff))
                    .unwrap()
                    .with_emission(0.)
                    .unwrap();
                let material = if configured {
                    material
                        .with_optics(black_box(OpticalProperties::default()))
                        .unwrap()
                } else {
                    material
                };
                black_box(material);
            }
            samples.push(started.elapsed() / iterations);
        }
        samples.sort();
        reports.push(samples[samples.len() / 2]);
    }
    eprintln!(
        "opaque construction {:?}; explicit default optics {:?}; material bytes {}",
        reports[0],
        reports[1],
        std::mem::size_of::<Material>()
    );
}
