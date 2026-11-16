use fraction::Fraction;
use sanim_voxel::{
    AmbientOcclusion, AoMode, Environment, Light, Material, PointLight, SunLight, Volume,
    layout::Coordinate,
};
use sanim_voxel_render::{
    Camera, Limits, Projection, RenderMode, RenderSettings, Renderer, ShadowMode, Shadows,
};
use std::sync::Arc;
fn coordinate(value: f64) -> Coordinate {
    Coordinate::Cells(Fraction::from(value))
}
fn camera() -> Camera {
    Camera {
        position: [0.5, 0.5, 8.],
        target: [0.5, 0.5, 0.],
        up: [0., 1., 0.],
        projection: Projection::Orthographic { height: 0.01 },
    }
}
fn options() -> RenderSettings {
    RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 1,
        height: 1,
        ambient: 0.,
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::Off,
            ..Default::default()
        },
        lighting: sanim_voxel::Lighting {
            environment_strength: 0.,
            shadows: Shadows {
                mode: ShadowMode::Off,
                ..Default::default()
            },
            ..Default::default()
        },
        background: [0., 0., 0., 1.],
        ..Default::default()
    }
}
fn volume(dimensions: [u32; 3]) -> Volume {
    Volume::new(dimensions)
        .unwrap()
        .fill_color([0; 3], [1; 3], 0xffffffff)
        .unwrap()
}
fn point(z: Coordinate, intensity: f32) -> Light {
    Light::Point(PointLight {
        position: [coordinate(0.5), coordinate(0.5), z],
        intensity,
        radius: 0.,
        ..Default::default()
    })
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn point_lights_obey_inverse_square_falloff_relative_coordinates_and_explicit_empty_override() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let near = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([point(coordinate(2.), 0.5)])),
        ..options()
    };
    let far = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([point(coordinate(3.), 0.5)])),
        ..options()
    };
    let a = renderer
        .render_rgba(&volume([1; 3]), &camera(), &near)
        .unwrap();
    let b = renderer
        .render_rgba(&volume([1; 3]), &camera(), &far)
        .unwrap();
    assert!(
        (i32::from(a.pixels[0]) - i32::from(b.pixels[0]) * 4).abs() <= 2,
        "near={:?} far={:?}",
        a.pixels,
        b.pixels
    );
    let relative = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([point(Coordinate::Percent(300.into()), 0.5)])),
        ..options()
    };
    let b = renderer
        .render_rgba(&volume([1; 3]), &camera(), &relative)
        .unwrap();
    let resized = renderer
        .render_rgba(&volume([1, 1, 2]), &camera(), &relative)
        .unwrap();
    assert!(
        resized.pixels[0] < b.pixels[0],
        "percent position must resolve against current volume dimensions"
    );
    let empty = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([])),
        ..options()
    };
    assert_eq!(
        renderer
            .render_rgba(&volume([1; 3]), &camera(), &empty)
            .unwrap()
            .pixels,
        [0, 0, 0, 255]
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn sampled_colored_lights_use_their_discrete_pdf_and_light_edits_do_not_reupload_voxels() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = volume([1; 3]);
    let make = |color| {
        Light::Sun(SunLight {
            direction: [0., 0., 1.],
            color,
            intensity: 1.,
            angular_diameter_degrees: 0.,
        })
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 256,
        bounces: 1,
        lights: Some(Arc::from([
            make([0.1, 0., 0.]),
            make([0., 0.2, 0.]),
            make([0., 0., 0.3]),
        ])),
        ..options()
    };
    let mixed = renderer.render_rgba(&volume, &camera(), &options).unwrap();
    let combined = renderer
        .render_rgba(
            &volume,
            &camera(),
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lights: Some(Arc::from([make([0.1, 0.2, 0.3])])),
                ..options.clone()
            },
        )
        .unwrap();
    for (a, b) in mixed.pixels[..3].iter().zip(&combined.pixels[..3]) {
        assert!(
            (i32::from(*a) - i32::from(*b)).abs() <= 20,
            "sampled={:?}, analytic={:?}",
            mixed.pixels,
            combined.pixels
        );
    }
    let mut previous = None;
    for step in 1..=64 {
        let lights: Arc<[Light]> = Arc::from([make([0.1 + step as f32 / 640., 0.2, 0.3])]);
        let weak = Arc::downgrade(&lights);
        let updated = RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            lights: Some(lights),
            ..options.clone()
        };
        let frame = renderer.render_rgba(&volume, &camera(), &updated).unwrap();
        assert_eq!(frame.stats.upload_bytes, 0);
        assert_eq!(frame.stats.buffer_allocations, 0);
        assert_eq!(frame.stats.texture_allocations, 0);
        assert_eq!(frame.stats.resident_bytes, combined.stats.resident_bytes);
        assert!(frame.stats.light_upload_bytes > 0);
        assert_eq!(
            renderer
                .render_rgba(&volume, &camera(), &updated)
                .unwrap()
                .stats
                .light_upload_bytes,
            0
        );
        if let Some(previous) = previous {
            assert!(
                std::sync::Weak::<[Light]>::upgrade(&previous).is_none(),
                "replaced light recipes must be released"
            );
        }
        previous = Some(weak);
    }
    renderer.release_volume().unwrap();
    assert!(previous.unwrap().upgrade().is_none());
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn environment_visibility_is_explicit_and_does_not_change_voxel_resources() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let empty = Volume::new([1; 3]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        environment: Some(Environment {
            top: [0.2, 0.4, 0.8],
            horizon: [0.2, 0.4, 0.8],
            ground: [0.2, 0.4, 0.8],
            visible: false,
            ..Default::default()
        }),
        ..options()
    };
    let hidden = renderer.render_rgba(&empty, &camera(), &options).unwrap();
    assert_eq!(hidden.pixels, [0, 0, 0, 255]);
    let visible = renderer
        .render_rgba(
            &empty,
            &camera(),
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                environment: options.environment.map(|environment| Environment {
                    visible: true,
                    ..environment
                }),
                ..options
            },
        )
        .unwrap();
    assert!(
        visible.pixels[0] > 0
            && visible.pixels[0] < visible.pixels[1]
            && visible.pixels[1] < visible.pixels[2]
    );
    assert_eq!(visible.stats.upload_bytes, 0);
    assert_eq!(visible.stats.light_upload_bytes, 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn point_shadow_rays_stop_at_the_light_and_absorb_only_the_traveled_liquid_distance() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let base = Volume::new([1, 1, 8])
        .unwrap()
        .fill_color([0; 3], [1; 3], 0xffffffff)
        .unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([point(coordinate(3.), 1.)])),
        lighting: sanim_voxel::Lighting {
            shadows: Shadows {
                mode: ShadowMode::Hard,
                ..Default::default()
            },
            ..options().lighting
        },
        ..options()
    };
    let view = Camera {
        position: [0.5, 0.5, 1.5],
        ..camera()
    };
    let open = renderer.render_rgba(&base, &view, &options).unwrap();
    let behind = base.fill_color([0, 0, 5], [1, 1, 6], 0xffffffff).unwrap();
    assert_eq!(
        open.pixels,
        renderer
            .render_rgba(&behind, &view, &options)
            .unwrap()
            .pixels,
        "geometry behind a point light cannot occlude it"
    );
    let (water, material) = base
        .with_material(Arc::new(
            sanim_voxel::Material::solid(0xffffffff)
                .unwrap()
                .with_optics(sanim_voxel::OpticalProperties {
                    transmission: 1.,
                    ior: 1.,
                    attenuation_color: [0.25, 0.5, 1.],
                    attenuation_distance: 2.,
                    ..Default::default()
                })
                .unwrap(),
        ))
        .unwrap();
    let water = water.fill_box([0, 0, 2], [1, 1, 8], material).unwrap();
    let submerged = renderer
        .render_rgba(
            &water,
            &view,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lights: Some(Arc::from([point(coordinate(4.), 9.)])),
                ..options
            },
        )
        .unwrap();
    for (actual, expected) in submerged.pixels[..3].iter().zip([64, 128, 255]) {
        assert!(
            (i32::from(*actual) - expected).abs() <= 1,
            "submerged light={:?}",
            submerged.pixels
        );
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn spherical_point_sources_produce_a_real_penumbra() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = Volume::new([64, 32, 32])
        .unwrap()
        .fill_color([0; 3], [64, 1, 32], 0xffffffff)
        .unwrap()
        .fill_color([26, 13, 0], [30, 15, 32], 0xffffffff)
        .unwrap();
    let view = Camera::fit(
        sanim_voxel_render::View::Top,
        volume.dimensions(),
        [257, 129],
    )
    .unwrap();
    let make = |radius| {
        Light::Point(PointLight {
            position: [coordinate(48.), coordinate(30.), coordinate(16.)],
            intensity: 600.,
            radius,
            ..Default::default()
        })
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 257,
        height: 129,
        mode: RenderMode::PathTraced,
        samples: 64,
        bounces: 1,
        lights: Some(Arc::from([make(0.)])),
        lighting: sanim_voxel::Lighting {
            shadows: Shadows {
                mode: ShadowMode::Soft,
                ..Default::default()
            },
            ..options().lighting
        },
        ..options()
    };
    let hard = renderer.render_rgba(&volume, &view, &options).unwrap();
    let soft = renderer
        .render_rgba(
            &volume,
            &view,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lights: Some(Arc::from([make(5.)])),
                ..options.clone()
            },
        )
        .unwrap();
    let mut penumbra = 0;
    for x in 0..options.width {
        let ray = view
            .ray_for_pixel([options.width, options.height], [x, options.height / 2])
            .unwrap();
        let Some(hit) = sanim_voxel::math::raycast(&volume, ray.origin, ray.direction).unwrap()
        else {
            continue;
        };
        if hit.position[1] != 0 {
            continue;
        }
        let offset = ((options.height / 2 * options.width + x) * 4) as usize;
        penumbra += usize::from(hard.pixels[offset] < 4 && soft.pixels[offset] > 12);
    }
    assert!(
        penumbra > 3,
        "spherical area source should illuminate the hard shadow's penumbra: {penumbra}"
    );
}

#[test]
#[ignore = "GPU scene-light performance benchmark"]
fn scene_light_benchmark() {
    use std::time::Instant;
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = Volume::new([128, 64, 128])
        .unwrap()
        .fill_color([0; 3], [128, 2, 128], 0xb0b0b0ff)
        .unwrap()
        .fill_color([56, 2, 40], [72, 34, 56], 0x708060ff)
        .unwrap();
    let view = Camera::fit(
        sanim_voxel_render::View::Isometric,
        volume.dimensions(),
        [640, 360],
    )
    .unwrap();
    for mode in [RenderMode::Flat, RenderMode::PathTraced] {
        for count in [1, sanim_voxel::MAX_LIGHTS] {
            let light = Light::Sun(SunLight {
                intensity: 1.1 / count as f32,
                angular_diameter_degrees: 0.,
                ..Default::default()
            });
            let options = RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                width: 640,
                height: 360,
                mode,
                samples: 64,
                bounces: 4,
                lights: Some(vec![light; count].into()),
                ..Default::default()
            };
            let first = renderer.render_rgba(&volume, &view, &options).unwrap();
            let warm = Instant::now();
            while warm.elapsed().as_millis() < 500 {
                renderer.render_rgba(&volume, &view, &options).unwrap();
            }
            let mut times = Vec::new();
            for _ in 0..9 {
                let start = Instant::now();
                let frame = renderer.render_rgba(&volume, &view, &options).unwrap();
                times.push(start.elapsed().as_micros());
                assert_eq!(frame.stats.upload_bytes, 0);
                assert_eq!(frame.stats.light_upload_bytes, 0);
                assert_eq!(frame.stats.resident_bytes, first.stats.resident_bytes);
            }
            times.sort_unstable();
            println!(
                "mode={mode:?} lights={count} median_us={} min_us={} max_us={} resident_bytes={}",
                times[4], times[0], times[8], first.stats.resident_bytes
            );
        }
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn flat_shadowless_preview_sums_colored_lights_analytically_without_selection_noise() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let make = |color| {
        Light::Sun(SunLight {
            direction: [0., 0., 1.],
            color,
            intensity: 1.,
            angular_diameter_degrees: 0.,
        })
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lights: Some(Arc::from([
            make([0.1, 0., 0.]),
            make([0., 0.2, 0.]),
            make([0., 0., 0.3]),
        ])),
        ..options()
    };
    let result = renderer
        .render_rgba(&volume([1; 3]), &camera(), &options)
        .unwrap();
    for (actual, expected) in result.pixels[..3].iter().zip([26, 51, 77]) {
        assert!((i32::from(*actual) - expected).abs() <= 1);
    }
    for seed in [1, 2, u32::MAX] {
        assert_eq!(
            result.pixels,
            renderer
                .render_rgba(
                    &volume([1; 3]),
                    &camera(),
                    &RenderSettings {
                        reconstruction: sanim_voxel_render::Reconstruction {
                            mode: sanim_voxel_render::ReconstructionMode::Off,
                            antialias_samples: 1,
                            ..Default::default()
                        },
                        seed,
                        ..options.clone()
                    }
                )
                .unwrap()
                .pixels
        );
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn finite_bulb_radius_must_extend_outside_opaque_lantern_geometry() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let (volume, emitter) = Volume::new([4, 1, 8])
        .unwrap()
        .with_material(Arc::new(
            Material::solid(0xffffffff)
                .unwrap()
                .with_emission(2.)
                .unwrap(),
        ))
        .unwrap();
    let volume = volume
        .fill_color([0; 3], [4, 1, 1], 0xffffffff)
        .unwrap()
        .fill_box([1, 0, 3], [2, 1, 4], emitter)
        .unwrap();
    let view = Camera {
        position: [1.5, 0.5, 1.5],
        target: [1.5, 0.5, 0.],
        ..camera()
    };
    let make = |radius| {
        Light::Point(PointLight {
            position: [coordinate(1.5), coordinate(0.5), coordinate(3.5)],
            intensity: 4.,
            radius,
            ..Default::default()
        })
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lighting: sanim_voxel::Lighting {
            shadows: Shadows {
                mode: ShadowMode::Soft,
                ..Default::default()
            },
            ..options().lighting
        },
        lights: Some(Arc::from([make(0.25)])),
        ..options()
    };
    let blocked = renderer.render_rgba(&volume, &view, &options).unwrap();
    let exposed = renderer
        .render_rgba(
            &volume,
            &view,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lights: Some(Arc::from([make(1.2)])),
                ..options
            },
        )
        .unwrap();
    assert_eq!(
        blocked.pixels[0], 0,
        "a source enclosed inside an opaque block cannot leak light"
    );
    assert!(
        exposed.pixels[0] > 20,
        "a finite emitter extending outside its bulb should illuminate nearby surfaces"
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn explicit_environment_controls_flat_ambient_without_changing_the_visible_backdrop() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let source = volume([1; 3]);
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        ambient: 1.,
        lights: Some(Arc::from([])),
        environment: Some(Environment {
            visible: false,
            top: [0.2, 0.4, 0.8],
            horizon: [0.2, 0.4, 0.8],
            ground: [0.2, 0.4, 0.8],
            ..Default::default()
        }),
        ..options()
    };
    let day = renderer.render_rgba(&source, &camera(), &options).unwrap();
    let night = renderer
        .render_rgba(
            &source,
            &camera(),
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                environment: options.environment.map(|environment| Environment {
                    intensity: 0.,
                    ..environment
                }),
                ..options
            },
        )
        .unwrap();
    assert!(day.pixels[0] > 0 && day.pixels[0] < day.pixels[1] && day.pixels[1] < day.pixels[2]);
    assert_eq!(night.pixels, [0, 0, 0, 255]);
    assert_eq!(night.stats.upload_bytes, 0);
    assert_eq!(night.stats.light_upload_bytes, 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn flat_soft_shadows_sum_each_colored_light_without_source_selection_spikes() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let make = |color| {
        Light::Sun(SunLight {
            direction: [0., 0., 1.],
            color,
            intensity: 1.,
            angular_diameter_degrees: 0.,
        })
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        lighting: sanim_voxel::Lighting {
            shadows: Shadows {
                mode: ShadowMode::Soft,
                samples: 4,
                ..Default::default()
            },
            ..options().lighting
        },
        lights: Some(Arc::from([
            make([0.1, 0., 0.]),
            make([0., 0.2, 0.]),
            make([0., 0., 0.3]),
        ])),
        ..options()
    };
    for seed in [0, 1, 2, u32::MAX] {
        let frame = renderer
            .render_rgba(
                &volume([1; 3]),
                &camera(),
                &RenderSettings {
                    reconstruction: sanim_voxel_render::Reconstruction {
                        mode: sanim_voxel_render::ReconstructionMode::Off,
                        antialias_samples: 1,
                        ..Default::default()
                    },
                    seed,
                    ..options.clone()
                },
            )
            .unwrap();
        for (actual, expected) in frame.pixels[..3].iter().zip([26, 51, 77]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 1,
                "seed={seed} frame={:?}",
                frame.pixels
            );
        }
    }
}

#[test]
fn all_light_flat_visibility_work_is_bounded_before_creating_a_device() {
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 3840,
        height: 2160,
        lighting: sanim_voxel::Lighting {
            shadows: Shadows {
                mode: ShadowMode::Soft,
                samples: 8,
                ..Default::default()
            },
            ..Default::default()
        },
        lights: Some(vec![Light::Sun(SunLight::default()); sanim_voxel::MAX_LIGHTS].into()),
        ..Default::default()
    };
    assert!(
        options
            .validate()
            .unwrap_err()
            .contains("visibility-ray budget")
    );
}
