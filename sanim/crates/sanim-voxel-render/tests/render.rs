use sanim_voxel::{Material, Texture, Volume};
use sanim_voxel_render::{Camera, Limits, RenderMode, RenderSettings, Renderer, View};
use std::{sync::Arc, time::Instant};

fn settings() -> RenderSettings {
    RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 65,
        height: 65,
        ambient: 1.0,
        ambient_occlusion: sanim_voxel_render::AmbientOcclusion {
            mode: sanim_voxel_render::AoMode::Off,
            ..Default::default()
        },
        background: [0.0; 4],
        ..Default::default()
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn flat_gpu_matches_color_blocks_and_retains_unchanged_uploads() {
    let mut renderer = Renderer::headless(Limits::default()).expect("GPU adapter");
    let volume = Volume::new([16; 3])
        .unwrap()
        .fill_color([4; 3], [12; 3], 0x3584e4ff)
        .unwrap();
    let options = settings();
    let camera = Camera::fit(
        View::Front,
        volume.dimensions(),
        [options.width, options.height],
    )
    .unwrap();
    let first = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let center = ((options.width / 2 + options.width * (options.height / 2)) * 4) as usize;
    assert_eq!(&first.pixels[center..center + 4], &[0x35, 0x84, 0xe4, 0xff]);
    assert_eq!(&first.pixels[..4], &[0; 4]);
    let second = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert_eq!(second.stats.upload_bytes, 0);
    assert_eq!(second.stats.buffer_allocations, 0);
    let material = volume.get([4; 3]).unwrap();
    let edited = volume.set([4; 3], 0).unwrap();
    let changed = renderer.render_rgba(&edited, &camera, &options).unwrap();
    assert_eq!(changed.stats.uploaded_chunks, 1);
    assert_eq!(changed.stats.updated_pages, 1);
    assert_eq!(changed.stats.buffer_allocations, 0);
    assert!(material != 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn color_only_rebuild_updates_palette_without_uploading_uniform_pages() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let base = Volume::new([16; 3]).unwrap();
    let red = base.fill_color([0; 3], [16; 3], 0xff0000ff).unwrap();
    let blue = base.fill_color([0; 3], [16; 3], 0x0000ffff).unwrap();
    assert_ne!(red.storage_identity(), blue.storage_identity());
    let camera = Camera::fit(View::Front, [16; 3], [65; 2]).unwrap();
    let options = settings();
    let red_frame = renderer.render_rgba(&red, &camera, &options).unwrap();
    let blue_frame = renderer.render_rgba(&blue, &camera, &options).unwrap();
    let center = (32 + 32 * 65) * 4;
    assert_eq!(&red_frame.pixels[center..center + 4], &[255, 0, 0, 255]);
    assert_eq!(&blue_frame.pixels[center..center + 4], &[0, 0, 255, 255]);
    assert_eq!(blue_frame.stats.uploaded_chunks, 0);
    assert_eq!(blue_frame.stats.updated_pages, 0);
    assert!(blue_frame.stats.upload_bytes < red_frame.stats.upload_bytes);
    assert_eq!(
        red_frame.stats.resident_bytes,
        blue_frame.stats.resident_bytes
    );

    // A real material-ID change still updates the page table.
    let changed = red.fill_color([0; 3], [16; 3], 0x0000ffff).unwrap();
    let frame = renderer.render_rgba(&changed, &camera, &options).unwrap();
    assert_eq!(frame.stats.updated_pages, 8);
    assert_eq!(&frame.pixels[center..center + 4], &[0, 0, 255, 255]);

    // Matching the first cell of a dense brick does not prove uniform contents.
    let base = Volume::new([8; 3]).unwrap();
    let sparse = base.fill_color([0; 3], [1; 3], 0xff0000ff).unwrap();
    let filled = base.fill_color([0; 3], [8; 3], 0xff0000ff).unwrap();
    let camera = Camera::fit(View::Front, [8; 3], [65; 2]).unwrap();
    renderer.render_rgba(&sparse, &camera, &options).unwrap();
    let frame = renderer.render_rgba(&filled, &camera, &options).unwrap();
    assert_eq!(frame.stats.updated_pages, 1);
    assert_eq!(&frame.pixels[center..center + 4], &[255, 0, 0, 255]);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn each_face_uses_its_png_tile_and_path_tracing_is_seeded() {
    let faces = [
        0xff0000ffu32,
        0x00ff00ff,
        0x0000ffff,
        0xffff00ff,
        0xff00ffff,
        0x00ffffff,
    ]
    .map(|color| {
        Some(Arc::new(
            Texture::new(1, 1, Arc::from(color.to_be_bytes())).unwrap(),
        ))
    });
    let (volume, material) = Volume::new([1; 3])
        .unwrap()
        .with_material(Arc::new(Material::textured(0xffffffff, faces).unwrap()))
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], material).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).expect("GPU adapter");
    let options = settings();
    for (view, expected) in [
        (View::Right, [255, 0, 0, 255]),
        (View::Left, [0, 255, 0, 255]),
        (View::Top, [0, 0, 255, 255]),
        (View::Bottom, [255, 255, 0, 255]),
        (View::Front, [255, 0, 255, 255]),
        (View::Back, [0, 255, 255, 255]),
    ] {
        let camera =
            Camera::fit(view, volume.dimensions(), [options.width, options.height]).unwrap();
        let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
        let center = ((options.width / 2 + options.width * (options.height / 2)) * 4) as usize;
        assert_eq!(&frame.pixels[center..center + 4], &expected, "{view}");
    }
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 4,
        bounces: 3,
        seed: 42,
        ..options
    };
    let camera = Camera::fit(
        View::Isometric,
        volume.dimensions(),
        [options.width, options.height],
    )
    .unwrap();
    let first = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let second = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert_eq!(first.pixels, second.pixels);
    assert!(first.pixels.iter().any(|value| *value != 0));
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn far_and_grazing_rays_progress_through_empty_bricks() {
    let volume = Volume::new([1024; 3])
        .unwrap()
        .fill_color([512, 512, 1], [513, 513, 2], 0xffffffff)
        .unwrap();
    let mut renderer = Renderer::headless(Limits::default()).expect("GPU adapter");
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 1,
        height: 1,
        ..settings()
    };
    for position in [
        [512.5, 512.5, 3000.0],
        [512.49, 512.5, 3000.0],
        [512.5, 512.5, 1024.0],
    ] {
        let camera = Camera {
            position,
            target: [512.5, 512.5, 1.5],
            up: [0.0, 1.0, 0.0],
            projection: sanim_voxel::Projection::Orthographic { height: 1.0 },
        };
        let ray = camera.ray_for_pixel([1; 2], [0; 2]).unwrap();
        let expected = sanim_voxel::math::raycast(&volume, ray.origin, ray.direction)
            .unwrap()
            .unwrap();
        assert_eq!(expected.position, [512, 512, 1]);
        let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert_eq!(frame.pixels, [255; 4], "camera at {position:?}");
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn cutout_tile_edges_match_cpu_picking_and_extreme_emission_saturates() {
    let texture = Arc::new(Texture::new(2, 1, Arc::from([255, 0, 0, 0, 0, 255, 0, 255])).unwrap());
    let material = Arc::new(
        Material::textured(0xffffffff, std::array::from_fn(|_| Some(texture.clone()))).unwrap(),
    );
    let (volume, id) = Volume::new([1; 3])
        .unwrap()
        .with_material(material)
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], id).unwrap();
    let camera = Camera {
        position: [2.0, 0.5, 0.0],
        target: [0.0, 0.5, 0.0],
        up: [0.0, 1.0, 0.0],
        projection: sanim_voxel::Projection::Orthographic { height: 1.0 },
    };
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 1,
        height: 1,
        ..settings()
    };
    let ray = camera.ray_for_pixel([1; 2], [0; 2]).unwrap();
    let hit = sanim_voxel::math::raycast(&volume, ray.origin, ray.direction)
        .unwrap()
        .unwrap();
    assert_eq!(hit.color, 0x00ff00ff);
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    assert_eq!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .pixels,
        [0, 255, 0, 255]
    );
    let material = Arc::new(
        Material::solid(0xffffffff)
            .unwrap()
            .with_emission(1e30)
            .unwrap(),
    );
    let (volume, id) = Volume::new([1; 3])
        .unwrap()
        .with_material(material)
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], id).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 1,
        bounces: 1,
        lighting: sanim_voxel_render::Lighting {
            key_direction: [f32::MAX, 0.0, 0.0],
            ..Default::default()
        },
        ..options
    };
    let mut camera = Camera::fit(View::Front, [1; 3], [1; 2]).unwrap();
    // Test emission, not stochastic edge coverage: the entire jitter window
    // must remain inside the white emitter's front face.
    camera.projection = sanim_voxel::Projection::Orthographic { height: 0.5 };
    assert_eq!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .pixels,
        [255; 4]
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn asynchronous_palette_edits_release_old_snapshots_and_bound_resident_bytes() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 64,
        height: 64,
        mode: RenderMode::PathTraced,
        samples: 16,
        bounces: 4,
        ..settings()
    };
    let camera = Camera::fit(View::Isometric, [8; 3], [64; 2]).unwrap();
    let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Asynchronous voxel regression"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: sanim_voxel_render::COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    let mut previous = None::<std::sync::Weak<Texture>>;
    let mut resident = 0;
    let mut native_baseline = None;
    for index in 0..40u8 {
        let texture = Arc::new(Texture::new(1, 1, Arc::from([index, 128, 255, 255])).unwrap());
        let weak = Arc::downgrade(&texture);
        let material = Arc::new(
            Material::textured(0xffffffff, std::array::from_fn(|_| Some(texture.clone()))).unwrap(),
        );
        let (volume, id) = Volume::new([8; 3])
            .unwrap()
            .with_material(material)
            .unwrap();
        let volume = volume.fill_box([0; 3], [8; 3], id).unwrap();
        let stats = renderer.render(&volume, &camera, &options, &view).unwrap();
        if resident != 0 {
            assert_eq!(resident, stats.resident_bytes);
        }
        resident = stats.resident_bytes;
        if let Some(previous) = previous {
            assert_eq!(previous.strong_count(), 0);
        }
        previous = Some(weak);
        if index == 10 {
            renderer.wait_idle().unwrap();
            native_baseline = renderer
                .device
                .generate_allocator_report()
                .map(|report| report.total_allocated_bytes);
        }
    }
    renderer.wait_idle().unwrap();
    if let (Some(baseline), Some(report)) =
        (native_baseline, renderer.device.generate_allocator_report())
    {
        assert!(
            report.total_allocated_bytes <= baseline + 4096,
            "native allocations grew from {baseline} to {}",
            report.total_allocated_bytes
        );
    }
    renderer.release_volume().unwrap();
    assert_eq!(previous.unwrap().strong_count(), 0);
}

#[test]
#[ignore = "GPU performance benchmark"]
fn voxel_gpu_benchmark() {
    let mut renderer = Renderer::headless(Limits::default()).expect("GPU adapter");
    println!("adapter={:?}", renderer.device.adapter_info());
    let volume = Volume::new([128; 3])
        .unwrap()
        .fill_color([4; 3], [124; 3], 0x3584e4ff)
        .unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 640,
        height: 360,
        ..settings()
    };
    let camera = Camera::fit(
        View::Isometric,
        volume.dimensions(),
        [options.width, options.height],
    )
    .unwrap();
    let cold = Instant::now();
    let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
    println!(
        "cold_us={} resident_bytes={} uploads={}",
        cold.elapsed().as_micros(),
        frame.stats.resident_bytes,
        frame.stats.upload_bytes
    );
    let start = Instant::now();
    for _ in 0..20 {
        let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert_eq!(frame.stats.upload_bytes, 0);
    }
    println!("warm_export_mean_us={}", start.elapsed().as_micros() / 20);
    let start = Instant::now();
    for _ in 0..10 {
        renderer.release_volume().unwrap();
        renderer.render_rgba(&volume, &camera, &options).unwrap();
    }
    println!(
        "uncached_volume_export_mean_us={}",
        start.elapsed().as_micros() / 10
    );
    let mut edited = volume.clone();
    let material = volume.get([4; 3]).unwrap();
    let start = Instant::now();
    let mut max_resident = 0;
    for index in 0..100 {
        edited = edited
            .set([4; 3], if index % 2 == 0 { 0 } else { material })
            .unwrap();
        let frame = renderer.render_rgba(&edited, &camera, &options).unwrap();
        assert!(frame.stats.uploaded_chunks <= 1);
        assert_eq!(frame.stats.buffer_allocations, 0);
        max_resident = max_resident.max(frame.stats.resident_bytes);
    }
    println!(
        "edit_export_mean_us={} max_resident_bytes={max_resident}",
        start.elapsed().as_micros() / 100
    );
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        mode: RenderMode::PathTraced,
        samples: 16,
        bounces: 4,
        background: [0.55, 0.72, 0.88, 1.0],
        sky_top: Some([0.25, 0.5, 0.85, 1.0]),
        ..options
    };
    let start = Instant::now();
    renderer.render_rgba(&volume, &camera, &options).unwrap();
    println!(
        "path_16samples_4bounces_export_us={}",
        start.elapsed().as_micros()
    );
}

#[test]
#[ignore = "GPU color hot-reload benchmark"]
fn color_hot_reload_benchmark() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 65,
        height: 65,
        ..settings()
    };
    for edge in [128, 256] {
        renderer.release_volume().unwrap();
        let base = Volume::new([edge; 3]).unwrap();
        let camera = Camera::fit(View::Front, base.dimensions(), [65; 2]).unwrap();
        let initial = base.fill_color([0; 3], [edge; 3], 0xff0000ff).unwrap();
        let first = renderer.render_rgba(&initial, &camera, &options).unwrap();
        let mut uploads = 0;
        let mut pages = 0;
        let mut build_us = 0;
        let mut render_us = 0;
        for index in 0..20 {
            let start = Instant::now();
            let color = if index % 2 == 0 {
                0x0000ffff
            } else {
                0xff0000ff
            };
            let changed = base.fill_color([0; 3], [edge; 3], color).unwrap();
            build_us += start.elapsed().as_micros();
            let start = Instant::now();
            let frame = renderer.render_rgba(&changed, &camera, &options).unwrap();
            render_us += start.elapsed().as_micros();
            assert_eq!(frame.stats.resident_bytes, first.stats.resident_bytes);
            let center = (32 + 32 * 65) * 4;
            assert_eq!(&frame.pixels[center..center + 4], &color.to_be_bytes());
            uploads += frame.stats.upload_bytes;
            pages += frame.stats.updated_pages;
        }
        println!(
            "edge={edge} mean_build_us={} mean_color_render_us={} mean_upload_bytes={} mean_page_updates={} resident_bytes={}",
            build_us / 20,
            render_us / 20,
            uploads / 20,
            pages / 20,
            first.stats.resident_bytes
        );
    }
}

#[test]
#[ignore = "GPU lighting performance benchmark"]
fn lighting_benchmark() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = Volume::new([128, 64, 128])
        .unwrap()
        .fill_color([0, 0, 0], [128, 2, 128], 0xb0b0b0ff)
        .unwrap()
        .fill_color([56, 2, 40], [72, 34, 56], 0x708060ff)
        .unwrap();
    let camera = Camera::fit(View::Isometric, volume.dimensions(), [640, 360]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 640,
        height: 360,
        mode: RenderMode::PathTraced,
        samples: 64,
        bounces: 4,
        ..Default::default()
    };
    let configurations = [
        ("path_default_64spp4b", options.clone()),
        (
            "path_ao4_64spp4b",
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                ambient_occlusion: sanim_voxel_render::AmbientOcclusion {
                    mode: sanim_voxel_render::AoMode::RayTraced,
                    ..Default::default()
                },
                ..options.clone()
            },
        ),
        (
            "path_soft4_64spp4b",
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                lighting: sanim_voxel_render::Lighting {
                    shadows: sanim_voxel_render::Shadows {
                        mode: sanim_voxel_render::ShadowMode::Soft,
                        samples: 4,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..options.clone()
            },
        ),
        (
            "flat_default",
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                mode: RenderMode::Flat,
                ..options.clone()
            },
        ),
        (
            "flat_hard",
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                mode: RenderMode::Flat,
                lighting: sanim_voxel_render::Lighting {
                    shadows: sanim_voxel_render::Shadows {
                        mode: sanim_voxel_render::ShadowMode::Hard,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..options.clone()
            },
        ),
        (
            "flat_soft4_ao4",
            RenderSettings {
                reconstruction: sanim_voxel_render::Reconstruction {
                    mode: sanim_voxel_render::ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                mode: RenderMode::Flat,
                lighting: sanim_voxel_render::Lighting {
                    shadows: sanim_voxel_render::Shadows {
                        mode: sanim_voxel_render::ShadowMode::Soft,
                        samples: 4,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ambient_occlusion: sanim_voxel_render::AmbientOcclusion {
                    mode: sanim_voxel_render::AoMode::RayTraced,
                    ..Default::default()
                },
                ..options
            },
        ),
    ];
    for (label, options) in configurations {
        renderer.render_rgba(&volume, &camera, &options).unwrap();
        let mut times = Vec::new();
        for _ in 0..9 {
            let start = Instant::now();
            let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
            assert_eq!(frame.stats.upload_bytes, 0);
            times.push(start.elapsed().as_micros());
        }
        times.sort_unstable();
        println!(
            "{label} median_us={} min_us={} max_us={}",
            times[4], times[0], times[8]
        );
    }
}
