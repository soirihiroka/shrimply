use fraction::Fraction;
use sanim_voxel::{AmbientOcclusion, AoMode, Material, OpticalProperties, Volume};
use sanim_voxel_render::{
    Camera, Lighting, Limits, Projection, RenderMode, RenderSettings, Renderer,
};
use std::sync::Arc;

fn camera(position: [f32; 3], target: [f32; 3]) -> Camera {
    Camera {
        position,
        target,
        up: [0., 1., 0.],
        projection: Projection::Orthographic { height: 0.01 },
    }
}
fn settings() -> RenderSettings {
    RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 1,
        height: 1,
        ambient: 1.,
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::Off,
            ..Default::default()
        },
        lighting: Lighting {
            key_strength: 0.,
            environment_strength: 0.,
            ..Default::default()
        },
        background: [1.; 4],
        ..Default::default()
    }
}
fn add_material(volume: Volume, optics: OpticalProperties) -> (Volume, u32) {
    volume
        .with_material(Arc::new(
            Material::solid(0xffffffff)
                .unwrap()
                .with_optics(optics)
                .unwrap(),
        ))
        .unwrap()
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn contiguous_media_have_only_external_interfaces_and_exact_beer_attenuation() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let optics = OpticalProperties {
        transmission: 1.,
        ior: 1.,
        attenuation_color: [0.25, 0.5, 1.],
        attenuation_distance: 2.,
        ..Default::default()
    };
    let (volume, material) = add_material(Volume::new([1, 1, 128]).unwrap(), optics);
    // A long uniform region crosses16 bricks, not128 optical surfaces. At128
    // units it must remain blue rather than exhausting the interface budget.
    let deep = volume.fill_box([0; 3], [1, 1, 128], material).unwrap();
    let view = camera([0.5, 0.5, 132.], [0.5, 0.5, 0.]);
    let frame = renderer.render_rgba(&deep, &view, &settings()).unwrap();
    assert_eq!(frame.pixels, [0, 0, 255, 255]);
    let shallow = volume.fill_box([0; 3], [1, 1, 2], material).unwrap();
    let frame = renderer.render_rgba(&shallow, &view, &settings()).unwrap();
    for (actual, transmittance) in frame.pixels[..3].iter().zip(optics.attenuation_color) {
        let expected = (transmittance.powf(1. / 2.2) * 255.).round() as i32;
        assert!(
            (i32::from(*actual) - expected).abs() <= 1,
            "{actual} != {expected}"
        );
    }
    // Starting inside the medium integrates only the remaining half unit and
    // must still produce the volume's exit interface to air.
    let inside = camera([0.5, 0.5, 0.5], [0.5, 0.5, -1.]);
    let frame = renderer
        .render_rgba(&shallow, &inside, &settings())
        .unwrap();
    for (actual, transmittance) in frame.pixels[..3].iter().zip(optics.attenuation_color) {
        let expected = (transmittance.powf(0.25 / 2.2) * 255.).round() as i32;
        assert!(
            (i32::from(*actual) - expected).abs() <= 1,
            "{actual} != {expected}"
        );
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn dielectric_paths_reflect_real_scene_geometry_and_do_not_add_diffuse_water() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let (volume, water) = add_material(
        Volume::new([1, 1, 8]).unwrap(),
        OpticalProperties {
            transmission: 1.,
            ior: 4.,
            ..Default::default()
        },
    );
    let (volume, emitter) = volume
        .with_material(Arc::new(
            Material::solid(0xff0000ff)
                .unwrap()
                .with_emission(4.)
                .unwrap(),
        ))
        .unwrap();
    let volume = volume
        .fill_box([0, 0, 2], [1, 1, 3], water)
        .unwrap()
        .fill_box([0, 0, 7], [1, 1, 8], emitter)
        .unwrap();
    let view = camera([0.5, 0.5, 6.], [0.5, 0.5, 0.]);
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 128,
        bounces: 1,
        background: [0., 0., 0., 1.],
        ..settings()
    };
    let frame = renderer.render_rgba(&volume, &view, &options).unwrap();
    assert!(
        frame.pixels[0] > 100,
        "a reflected ray must see the red emissive wall: {:?}",
        frame.pixels
    );
    assert_eq!(&frame.pixels[1..3], &[0, 0]);
    // White water with only red reflected light cannot invent a white Lambert term.
    let repeated = renderer.render_rgba(&volume, &view, &options).unwrap();
    assert_eq!(frame.pixels, repeated.pixels);
    assert_eq!(repeated.stats.upload_bytes, 0);
    assert_eq!(repeated.stats.buffer_allocations, 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn a_refracted_ray_reaches_a_different_landmark_than_an_index_matched_ray() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let build = |ior| {
        let (volume, water) = add_material(
            Volume::new([16, 1, 16]).unwrap(),
            OpticalProperties {
                transmission: 1.,
                ior,
                ..Default::default()
            },
        );
        let volume = volume.fill_box([0, 0, 7], [16, 1, 11], water).unwrap();
        let volume = volume.fill_color([0, 0, 0], [8, 1, 1], 0xff0000ff).unwrap();
        volume
            .fill_color([8, 0, 0], [16, 1, 1], 0x0000ffff)
            .unwrap()
    };
    // At the front surface x~10.6. The unbent ray reaches x~7.8 (red),
    // while Snell refraction through four cells reaches x~8.2 (blue).
    let view = camera([12., 0.5, 16.], [7.665, 0.5, 0.]);
    let clear = renderer
        .render_rgba(&build(1.), &view, &settings())
        .unwrap();
    let refracted = renderer
        .render_rgba(&build(1.5), &view, &settings())
        .unwrap();
    assert!(
        clear.pixels[0] > clear.pixels[2],
        "clear={:?}",
        clear.pixels
    );
    assert!(
        refracted.pixels[2] > refracted.pixels[0],
        "refracted={:?}",
        refracted.pixels
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn animated_world_space_waves_are_uniform_only_and_preserve_premultiplied_output() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let (volume, water) = add_material(
        Volume::new([16, 4, 16]).unwrap(),
        OpticalProperties {
            transmission: 1.,
            ior: 1.333,
            wave_strength: 0.5,
            wave_length: 4.,
            wave_speed: 1.,
            ..Default::default()
        },
    );
    let volume = volume.fill_box([0; 3], [16, 2, 16], water).unwrap();
    let mut view = camera([8., 6., 18.], [8., 1., 8.]);
    view.projection = Projection::Orthographic { height: 8. };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 65,
        height: 65,
        lighting: Lighting::default(),
        background: [0., 0., 0., 0.],
        opacity: 0.25,
        ..settings()
    };
    let first = renderer.render_rgba(&volume, &view, &options).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        time: Fraction::new(1u64, 2u64),
        ..options
    };
    let second = renderer.render_rgba(&volume, &view, &options).unwrap();
    assert!(
        first.pixels != second.pixels,
        "world-space surface normals should move with render time"
    );
    assert_eq!(second.stats.upload_bytes, 0);
    assert_eq!(second.stats.buffer_allocations, 0);
    assert_eq!(first.stats.resident_bytes, second.stats.resident_bytes);
    for pixel in second.pixels.as_chunks::<4>().0 {
        assert!(pixel[..3].iter().all(|channel| *channel <= pixel[3]));
    }
    assert_eq!(
        second.pixels,
        renderer
            .render_rgba(&volume, &view, &options)
            .unwrap()
            .pixels
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn total_internal_reflection_stays_in_the_medium_until_a_valid_exit() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let (volume, water) = add_material(
        Volume::new([8, 2, 8]).unwrap(),
        OpticalProperties {
            transmission: 1.,
            ior: 1.5,
            attenuation_color: [0.25, 1., 1.],
            ..Default::default()
        },
    );
    let volume = volume.fill_box([0; 3], [8, 2, 8], water).unwrap();
    // The nearby +Z interface is above the critical angle (cos=.6); the
    // reflected ray must travel to +X before it can escape (cos=.8).
    let view = camera([3., 1., 7.5], [3.8, 1., 8.1]);
    let frame = renderer.render_rgba(&volume, &view, &settings()).unwrap();
    assert!(
        frame.pixels[0] < 50 && frame.pixels[2] > 180,
        "TIR must retain the long, red-absorbing path: {:?}",
        frame.pixels
    );
    let path = renderer
        .render_rgba(
            &volume,
            &view,
            &RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                mode: RenderMode::PathTraced,
                samples: 64,
                bounces: 1,
                lighting: Lighting {
                    key_strength: 0.,
                    environment_upper: [1.; 3],
                    environment_horizon: [1.; 3],
                    environment_lower: [1.; 3],
                    ..Default::default()
                },
                ..settings()
            },
        )
        .unwrap();
    assert!(
        path.pixels[0] < path.pixels[2] / 2 && path.pixels[2] > 100,
        "path TIR: {:?}",
        path.pixels
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn transparent_shadow_connections_are_colored_and_palette_edits_retain_cells() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let build = |color| {
        let (volume, water) = add_material(
            Volume::new([8, 16, 24]).unwrap(),
            OpticalProperties {
                transmission: 1.,
                ior: 1.,
                attenuation_color: color,
                attenuation_distance: 2.,
                ..Default::default()
            },
        );
        volume
            .fill_box([0, 8, 8], [8, 16, 16], water)
            .unwrap()
            .fill_color([0; 3], [8, 16, 8], 0xffffffff)
            .unwrap()
    };
    let view = camera([4., 4., 20.], [4., 4., 0.]);
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        ambient: 0.,
        lighting: Lighting {
            key_direction: [0., 1., 1.],
            key_color: [1.; 3],
            key_strength: 1.,
            sun_diameter_degrees: 0.,
            environment_strength: 0.,
            shadows: sanim_voxel_render::Shadows {
                mode: sanim_voxel_render::ShadowMode::Hard,
                ..Default::default()
            },
            ..Default::default()
        },
        ..settings()
    };
    let clear = renderer
        .render_rgba(&build([1.; 3]), &view, &options)
        .unwrap();
    let tinted = renderer
        .render_rgba(&build([0.25, 0.5, 1.]), &view, &options)
        .unwrap();
    assert!(
        clear.pixels[0] > 150,
        "unobstructed clear shadow: {:?}",
        clear.pixels
    );
    assert!(
        tinted.pixels[0] < tinted.pixels[1] && tinted.pixels[1] < tinted.pixels[2],
        "colored shadow: {:?}",
        tinted.pixels
    );
    assert_eq!(tinted.pixels[2], clear.pixels[2]);
    assert_eq!(tinted.stats.uploaded_chunks, 0);
    assert_eq!(tinted.stats.resident_bytes, clear.stats.resident_bytes);
}

#[test]
fn invalid_or_unrepresentable_wave_clock_is_rejected_without_a_gpu() {
    for time in [
        Fraction::from(f64::INFINITY),
        Fraction::from(f64::NAN),
        Fraction::from(f32::MAX),
    ] {
        assert!(
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                time,
                ..Default::default()
            }
            .validate()
            .unwrap_err()
            .contains("wave-phase")
        );
    }
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            time: Fraction::new(1u64, 30u64),
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn rough_dielectric_sampling_remains_finite_from_nearly_smooth_to_frosted() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let view = camera([4., 4., 12.], [4., 4., 0.]);
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 64,
        bounces: 1,
        opacity: 0.5,
        lighting: Lighting {
            key_direction: [0., 0., 1.],
            key_strength: 1.,
            environment_upper: [1.; 3],
            environment_horizon: [1.; 3],
            environment_lower: [1.; 3],
            ..Default::default()
        },
        ..settings()
    };
    for roughness in [0., 0.0001, 0.001, 0.05, 0.5, 1.] {
        let (volume, glass) = add_material(
            Volume::new([8; 3]).unwrap(),
            OpticalProperties {
                transmission: 1.,
                ior: 1.5,
                roughness,
                ..Default::default()
            },
        );
        let volume = volume.fill_box([0; 3], [8; 3], glass).unwrap();
        let frame = renderer.render_rgba(&volume, &view, &options).unwrap();
        assert!(
            frame.pixels[0] > 0,
            "roughness={roughness}: {:?}",
            frame.pixels
        );
        assert!(
            frame.pixels[..3]
                .iter()
                .all(|value| *value <= frame.pixels[3])
        );
        assert_eq!(frame.pixels[0], frame.pixels[1]);
        assert_eq!(frame.pixels[1], frame.pixels[2]);
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn index_matched_glass_preserves_the_visible_backdrop_and_coverage_in_both_modes() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let empty = Volume::new([8; 3]).unwrap();
    let (glass, material) = add_material(
        empty.clone(),
        OpticalProperties {
            transmission: 1.,
            ior: 1.,
            roughness: 1.,
            ..Default::default()
        },
    );
    let glass = glass.fill_box([0; 3], [8; 3], material).unwrap();
    let view = camera([4., 4., 12.], [4., 4., 0.]);
    for mode in [RenderMode::Flat, RenderMode::PathTraced] {
        let options = RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            mode,
            samples: 16,
            background: [0.2, 0.4, 0.8, 0.3],
            lighting: Lighting {
                key_strength: 0.,
                environment_upper: [1., 0., 0.],
                environment_horizon: [1., 0., 0.],
                environment_lower: [1., 0., 0.],
                ..Default::default()
            },
            ..settings()
        };
        let backdrop = renderer.render_rgba(&empty, &view, &options).unwrap();
        let transmitted = renderer.render_rgba(&glass, &view, &options).unwrap();
        assert_eq!(
            backdrop.pixels, transmitted.pixels,
            "{mode:?} must not substitute the lighting environment for a directly transmitted backdrop"
        );
    }
}

#[test]
#[ignore = "GPU transmission performance benchmark"]
fn transmission_benchmark() {
    use std::time::Instant;
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let build = |transmission| {
        let (volume, water) = add_material(
            Volume::new([128, 32, 128]).unwrap(),
            OpticalProperties {
                transmission,
                ior: 1.333,
                roughness: 0.05,
                attenuation_color: [0.4, 0.8, 0.95],
                attenuation_distance: 16.,
                wave_strength: 0.1,
                wave_length: 16.,
                wave_speed: 0.4,
            },
        );
        volume
            .fill_box([0, 8, 0], [128, 16, 128], water)
            .unwrap()
            .fill_color([0; 3], [128, 8, 128], 0xb0a080ff)
            .unwrap()
            .fill_color([56, 16, 40], [72, 32, 56], 0x708060ff)
            .unwrap()
    };
    let view = Camera::fit(
        sanim_voxel_render::View::Isometric,
        [128, 32, 128],
        [640, 360],
    )
    .unwrap();
    for mode in [RenderMode::Flat, RenderMode::PathTraced] {
        for transmission in [0., 1.] {
            let volume = build(transmission);
            let mut options = RenderSettings {
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
                ..Default::default()
            };
            let first = renderer.render_rgba(&volume, &view, &options).unwrap();
            // Warm the pipeline and clock before measuring the same GPU/readback work.
            let warm = Instant::now();
            while warm.elapsed().as_millis() < 500 {
                renderer.render_rgba(&volume, &view, &options).unwrap();
            }
            let mut times = Vec::new();
            for frame in 0u64..9 {
                options.time = Fraction::new(frame, 30u64);
                let start = Instant::now();
                let rendered = renderer.render_rgba(&volume, &view, &options).unwrap();
                times.push(start.elapsed().as_micros());
                assert_eq!(rendered.stats.upload_bytes, 0);
                assert_eq!(rendered.stats.buffer_allocations, 0);
                assert_eq!(rendered.stats.texture_allocations, 0);
                assert_eq!(rendered.stats.resident_bytes, first.stats.resident_bytes);
            }
            times.sort_unstable();
            println!(
                "mode={mode:?} transmission={transmission} median_us={} min_us={} max_us={} resident_bytes={}",
                times[4], times[0], times[8], first.stats.resident_bytes
            );
        }
    }
}
