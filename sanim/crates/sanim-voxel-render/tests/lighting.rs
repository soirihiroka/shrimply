use sanim_voxel::{AmbientOcclusion, AoMode, Lighting, ShadowMode, Shadows, Volume};
use sanim_voxel_render::{Camera, Limits, RenderMode, RenderSettings, Renderer, View};

#[test]
fn light_basis_is_finite_and_sun_size_preserves_normal_incidence() {
    for direction in [
        [-0.4, 0.8, 0.5],
        [0.0, 1.0, 0.0],
        [f32::MAX, 0.0, 0.0],
        [f32::MIN_POSITIVE, 0.0, 0.0],
    ] {
        for diameter in [0.0, 8.0, 60.0, 180.0] {
            let lighting = Lighting {
                key_direction: direction,
                sun_diameter_degrees: diameter,
                ..Default::default()
            };
            let basis = lighting.parameters().unwrap();
            assert!(
                basis
                    .direction
                    .iter()
                    .chain(&basis.tangent)
                    .chain(&basis.bitangent)
                    .all(|component| component.is_finite())
            );
            let dot = basis
                .direction
                .iter()
                .zip(basis.tangent)
                .map(|(a, b)| a * b)
                .sum::<f32>();
            assert!(dot.abs() < 1e-6);
            assert!((basis.normalization * (1.0 + basis.cos_radius) * 0.5 - 1.0).abs() < 1e-6);
        }
    }
    assert!(
        Lighting {
            sun_diameter_degrees: 181.0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        Lighting {
            environment_horizon: [f32::NAN, 0.0, 0.0],
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        Lighting {
            key_color: [f32::MAX; 3],
            key_strength: f32::MAX,
            ..Default::default()
        }
        .validate()
        .unwrap_err()
        .contains("radiance range")
    );
    let extinguished = Lighting {
        environment_upper: [f32::MAX; 3],
        environment_strength: 0.0,
        ..Default::default()
    }
    .parameters()
    .unwrap();
    assert_eq!(extinguished.environment_upper, [0.0; 3]);
}

#[test]
fn automatic_modes_and_extra_visibility_work_are_bounded() {
    let options = RenderSettings::default();
    assert_eq!(options.effective_modes(), (ShadowMode::Off, AoMode::Local));
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        ..options
    };
    assert_eq!(options.effective_modes(), (ShadowMode::Soft, AoMode::Off));
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            ambient_occlusion: AmbientOcclusion {
                samples: 17,
                ..Default::default()
            },
            ..options.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            ambient_occlusion: AmbientOcclusion {
                mode: AoMode::RayTraced,
                distance: f32::INFINITY,
                ..Default::default()
            },
            ..options.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            samples: 64,
            ambient_occlusion: AmbientOcclusion {
                mode: AoMode::RayTraced,
                samples: 16,
                ..Default::default()
            },
            ..options.clone()
        }
        .validate()
        .unwrap_err()
        .contains("visibility-ray budget")
    );
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            exposure_ev: 17.0,
            ..options
        }
        .validate()
        .is_err()
    );
    for mode in ShadowMode::ALL {
        assert_eq!(mode.name().parse::<ShadowMode>().unwrap(), *mode);
    }
    for mode in AoMode::ALL {
        assert_eq!(mode.name().parse::<AoMode>().unwrap(), *mode);
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn surface_snap_does_not_teleport_a_ray_originating_inside_solid_material() {
    let volume = Volume::new([1; 3])
        .unwrap()
        .fill_color([0; 3], [1; 3], 0xffffffff)
        .unwrap();
    let camera = Camera {
        position: [0.5; 3],
        target: [0.5, 0.5, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: sanim_voxel::Projection::Orthographic { height: 0.1 },
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 1,
        height: 1,
        mode: RenderMode::PathTraced,
        samples: 1,
        bounces: 2,
        lighting: Lighting {
            key_strength: 0.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    assert_eq!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .pixels,
        [0, 0, 0, 255]
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn ray_ao_is_distance_limited_and_does_not_darken_direct_light_or_emission() {
    let volume = Volume::new([64, 32, 32])
        .unwrap()
        .fill_color([0, 0, 0], [64, 1, 32], 0xffffffff)
        .unwrap()
        .fill_color([24, 1, 0], [26, 18, 32], 0xffffffff)
        .unwrap();
    let camera = Camera::fit(View::Top, volume.dimensions(), [257, 129]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 257,
        height: 129,
        ambient: 1.0,
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::RayTraced,
            strength: 1.0,
            distance: 0.5,
            samples: 16,
        },
        ..Default::default()
    };
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let short = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let long = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                ambient_occlusion: AmbientOcclusion {
                    distance: 8.0,
                    ..options.ambient_occlusion
                },
                ..options.clone()
            },
        )
        .unwrap();
    let mut short_sum = 0u32;
    let mut long_sum = 0u32;
    for x in 0..options.width {
        let ray = camera
            .ray_for_pixel([options.width, options.height], [x, options.height / 2])
            .unwrap();
        let Some(hit) = sanim_voxel::math::raycast(&volume, ray.origin, ray.direction).unwrap()
        else {
            continue;
        };
        if hit.position[1] != 0 || !(20..23).contains(&hit.position[0]) {
            continue;
        }
        let offset = ((options.height / 2 * options.width + x) * 4) as usize;
        short_sum += u32::from(short.pixels[offset]);
        long_sum += u32::from(long.pixels[offset]);
    }
    assert!(
        long_sum < short_sum,
        "longer AO rays should see the nearby wall: short={short_sum}, long={long_sum}"
    );
    let direct_options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 16,
        bounces: 1,
        lighting: Lighting {
            shadows: Shadows {
                mode: ShadowMode::Off,
                ..Default::default()
            },
            environment_strength: 0.0,
            ..Default::default()
        },
        ..options.clone()
    };
    let no_ao = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                ambient_occlusion: AmbientOcclusion {
                    mode: AoMode::Off,
                    ..options.ambient_occlusion
                },
                ..direct_options.clone()
            },
        )
        .unwrap();
    let ray_ao = renderer
        .render_rgba(&volume, &camera, &direct_options)
        .unwrap();
    assert_eq!(
        no_ao.pixels, ray_ao.pixels,
        "artistic AO must not multiply direct sun illumination"
    );
    let (emissive, material) = Volume::new([1; 3])
        .unwrap()
        .with_material(std::sync::Arc::new(
            sanim_voxel::Material::solid(0xffffffff)
                .unwrap()
                .with_emission(2.0)
                .unwrap(),
        ))
        .unwrap();
    let emissive = emissive.fill_box([0; 3], [1; 3], material).unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [257, 129]).unwrap();
    let no_ao = renderer
        .render_rgba(
            &emissive,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                ambient_occlusion: AmbientOcclusion {
                    mode: AoMode::Off,
                    ..options.ambient_occlusion
                },
                ..direct_options.clone()
            },
        )
        .unwrap();
    let ray_ao = renderer
        .render_rgba(&emissive, &camera, &direct_options)
        .unwrap();
    assert_eq!(no_ao.pixels, ray_ao.pixels);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn environment_fill_is_independent_of_visible_background_and_keeps_premultiplied_alpha() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = Volume::new([1; 3])
        .unwrap()
        .fill_color([0; 3], [1; 3], 0xffffffff)
        .unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [65; 2]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 65,
        height: 65,
        mode: RenderMode::PathTraced,
        samples: 32,
        bounces: 2,
        background: [1.0, 0.0, 0.0, 1.0],
        lighting: Lighting {
            key_strength: 0.0,
            environment_upper: [0.25; 3],
            environment_horizon: [0.25; 3],
            environment_lower: [0.25; 3],
            ..Default::default()
        },
        ..Default::default()
    };
    let red = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let blue_options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        background: [0.0, 0.0, 1.0, 1.0],
        ..options.clone()
    };
    let blue = renderer
        .render_rgba(&volume, &camera, &blue_options)
        .unwrap();
    let center = (32 + 32 * 65) * 4;
    assert_eq!(
        &red.pixels[center..center + 4],
        &blue.pixels[center..center + 4]
    );
    assert!(
        red.pixels[center] > 100,
        "environment must illuminate the surface without a key light"
    );
    assert_eq!(&red.pixels[..4], &[255, 0, 0, 255]);
    assert_eq!(&blue.pixels[..4], &[0, 0, 255, 255]);
    assert_eq!(
        blue.stats.upload_bytes, 0,
        "lighting/background changes must retain voxel data"
    );
    let bright = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                exposure_ev: 1.0,
                ..options.clone()
            },
        )
        .unwrap();
    assert!(bright.pixels[center] > red.pixels[center]);
    assert_eq!(&bright.pixels[..4], &red.pixels[..4]);
    let faded = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                opacity: 0.5,
                ..options
            },
        )
        .unwrap();
    assert!((127..=128).contains(&faded.pixels[0]));
    assert_eq!(&faded.pixels[1..3], &[0, 0]);
    assert_eq!(faded.pixels[0], faded.pixels[3]);
    assert!(faded.pixels[center] <= faded.pixels[center + 3]);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn a_finite_sun_produces_a_penumbra_instead_of_a_hard_shadow() {
    let volume = Volume::new([64, 32, 32])
        .unwrap()
        .fill_color([0, 0, 0], [64, 1, 32], 0xffffffff)
        .unwrap()
        .fill_color([26, 13, 0], [30, 15, 32], 0xffffffff)
        .unwrap();
    let camera = Camera::fit(View::Top, volume.dimensions(), [257, 129]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 257,
        height: 129,
        mode: RenderMode::PathTraced,
        samples: 128,
        bounces: 1,
        lighting: Lighting {
            key_direction: [0.4, 1.0, 0.0],
            key_color: [1.0; 3],
            key_strength: 1.0,
            sun_diameter_degrees: 0.0,
            environment_strength: 1.0,
            environment_upper: [0.0; 3],
            environment_horizon: [0.0; 3],
            environment_lower: [0.0; 3],
            shadows: Default::default(),
        },
        ..Default::default()
    };
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let hard = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let soft = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lighting: Lighting {
                    sun_diameter_degrees: 24.0,
                    ..options.lighting
                },
                ..options.clone()
            },
        )
        .unwrap();
    let unshadowed = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lighting: Lighting {
                    shadows: Shadows {
                        mode: ShadowMode::Off,
                        ..Default::default()
                    },
                    ..options.lighting
                },
                ..options.clone()
            },
        )
        .unwrap();
    let half = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lighting: Lighting {
                    shadows: Shadows {
                        mode: ShadowMode::Hard,
                        strength: 0.5,
                        ..Default::default()
                    },
                    ..options.lighting
                },
                ..options.clone()
            },
        )
        .unwrap();
    let mut hard_midtones = 0;
    let mut soft_midtones = 0;
    let mut hard_sum = 0u32;
    let mut half_sum = 0u32;
    let mut unshadowed_sum = 0u32;
    for x in 0..options.width {
        let ray = camera
            .ray_for_pixel([options.width, options.height], [x, options.height / 2])
            .unwrap();
        let Some(hit) = sanim_voxel::math::raycast(&volume, ray.origin, ray.direction).unwrap()
        else {
            continue;
        };
        if hit.position[1] != 0 || !(10..26).contains(&hit.position[0]) {
            continue;
        }
        let offset = ((options.height / 2 * options.width + x) * 4) as usize;
        hard_midtones += usize::from((24..200).contains(&hard.pixels[offset]));
        soft_midtones += usize::from((24..200).contains(&soft.pixels[offset]));
        hard_sum += u32::from(hard.pixels[offset]);
        half_sum += u32::from(half.pixels[offset]);
        unshadowed_sum += u32::from(unshadowed.pixels[offset]);
    }
    assert!(
        soft_midtones > hard_midtones + 3,
        "finite sun must broaden the shadow transition: hard={hard_midtones}, soft={soft_midtones}"
    );
    assert_eq!(soft.stats.upload_bytes, 0);
    assert!(hard_sum < half_sum && half_sum < unshadowed_sum);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn flat_emission_is_clipped_before_element_opacity_to_preserve_premultiplication() {
    let (volume, material) = Volume::new([1; 3])
        .unwrap()
        .with_material(std::sync::Arc::new(
            sanim_voxel::Material::solid(0xffffffff)
                .unwrap()
                .with_emission(10.)
                .unwrap(),
        ))
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], material).unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [1; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let frame = renderer
        .render_rgba(
            &volume,
            &camera,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                width: 1,
                height: 1,
                opacity: 0.25,
                ..Default::default()
            },
        )
        .unwrap();
    assert!((63..=64).contains(&frame.pixels[3]));
    assert!(
        frame.pixels[..3]
            .iter()
            .all(|color| *color == frame.pixels[3])
    );
}
