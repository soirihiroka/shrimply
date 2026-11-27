use sanim_voxel::{Camera, Material, ShadowMode, View, Volume};
use sanim_voxel_render::{
    AmbientOcclusion, AoMode, Limits, Reconstruction, ReconstructionMode, RenderMode,
    RenderSettings, Renderer,
};
use std::sync::Arc;

fn settings(mode: ReconstructionMode) -> RenderSettings {
    RenderSettings {
        width: 64,
        height: 64,
        background: [0.; 4],
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::Off,
            ..Default::default()
        },
        reconstruction: Reconstruction {
            mode,
            antialias_samples: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn volume() -> Volume {
    Volume::new([16; 3])
        .unwrap()
        .fill_color([2; 3], [14; 3], 0x3584e4ff)
        .unwrap()
}

#[test]
fn reconstruction_settings_are_bounded_and_names_round_trip() {
    for mode in ReconstructionMode::ALL {
        assert_eq!(mode.name().parse::<ReconstructionMode>().unwrap(), mode);
    }
    let mut options = settings(ReconstructionMode::Spatial);
    options.reconstruction.spatial_passes = 4;
    assert!(options.validate().unwrap_err().contains("spatial"));
    options.reconstruction = Reconstruction {
        antialias_samples: 0,
        ..Default::default()
    };
    assert!(options.validate().unwrap_err().contains("antialias"));
    options.reconstruction = Reconstruction {
        history_frames: 33,
        ..Default::default()
    };
    assert!(options.validate().unwrap_err().contains("history"));
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn spatial_preserves_flat_colors_alpha_and_reuses_bounded_targets() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = volume();
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    let mut options = settings(ReconstructionMode::Off);
    options.ambient = 1.;
    options.lighting.shadows.mode = ShadowMode::Soft;
    let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
    options.reconstruction.mode = ReconstructionMode::Spatial;
    let filtered = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert!(
        raw.pixels
            .iter()
            .zip(&filtered.pixels)
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
    assert_eq!(filtered.stats.reconstruction_bytes, 64 * 64 * 48 + 1280);
    let warm = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert_eq!(warm.stats.buffer_allocations, 0);
    assert_eq!(warm.stats.texture_allocations, 0);
    assert_eq!(warm.stats.upload_bytes, 0);
    assert_eq!(filtered.stats.resident_bytes, warm.stats.resident_bytes);
    options.opacity = 0.25;
    let quarter = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert!(
        quarter
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[..3].iter().all(|v| *v <= pixel[3]))
    );
    let center = (32 + 32 * 64) * 4;
    assert_eq!(quarter.pixels[center + 3], 64);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn temporal_history_resets_on_edits_seeks_lighting_and_explicit_epochs() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = volume();
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    let mut options = settings(ReconstructionMode::Temporal);
    options.reconstruction.history_frames = 3;
    let first = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert!(first.stats.history_reset);
    assert_eq!(
        first.stats.reconstruction_bytes,
        64 * 64 * 64 + 1280,
        "deterministic temporal lighting must not allocate unused spatial scratch"
    );
    for expected in [2, 3, 3, 3] {
        let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert_eq!(frame.stats.history_frames, expected);
        assert!(!frame.stats.history_reset);
        assert_eq!(frame.stats.resident_bytes, first.stats.resident_bytes);
        assert_eq!(frame.stats.texture_allocations, 0);
        assert_eq!(frame.stats.upload_bytes, 0);
    }
    options.time = fraction::Fraction::from(2);
    assert!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .stats
            .history_reset
    );
    options.time = fraction::Fraction::from(0);
    let backward = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert!(backward.stats.history_reset);
    assert_eq!(first.pixels, backward.pixels);
    options.ambient = 0.2;
    assert!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .stats
            .history_reset
    );
    let edited = volume.set([2; 3], 0).unwrap();
    assert!(
        renderer
            .render_rgba(&edited, &camera, &options)
            .unwrap()
            .stats
            .history_reset
    );
    renderer.reset_history();
    assert!(
        renderer
            .render_rgba(&edited, &camera, &options)
            .unwrap()
            .stats
            .history_reset
    );
    options.time = fraction::Fraction::new(1u64, u64::MAX);
    renderer.render_rgba(&edited, &camera, &options).unwrap();
    options.time = fraction::Fraction::from(u64::MAX);
    assert!(
        renderer
            .render_rgba(&edited, &camera, &options)
            .unwrap()
            .stats
            .history_reset,
        "an unrepresentable exact time difference must reject history safely"
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn transmission_emission_and_visible_background_bypass_diffuse_filtering() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let base = Volume::new([8; 3]).unwrap();
    let material = Arc::new(
        Material::solid(0x62a0eaff)
            .unwrap()
            .with_emission(4.)
            .unwrap(),
    );
    let (base, id) = base.with_material(material).unwrap();
    let volume = base.fill_box([2; 3], [6; 3], id).unwrap();
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    for mode in [RenderMode::Flat, RenderMode::PathTraced] {
        let mut options = settings(ReconstructionMode::Off);
        options.mode = mode;
        options.samples = 4;
        options.background = [0.2, 0.1, 0.3, 0.4];
        options.lighting.shadows.mode = ShadowMode::Off;
        let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
        options.reconstruction.mode = ReconstructionMode::Spatial;
        let reconstructed = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert!(
            raw.pixels
                .iter()
                .zip(&reconstructed.pixels)
                .all(|(a, b)| a.abs_diff(*b) <= 1)
        );
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn black_checker_texels_do_not_darken_white_neighbors_or_lose_contrast() {
    let bytes = (0..16 * 16)
        .flat_map(|index| {
            let c = if (index % 16 + index / 16) % 2 == 0 {
                255
            } else {
                0
            };
            [c, c, c, 255]
        })
        .collect::<Vec<_>>();
    let texture = Arc::new(sanim_voxel::Texture::new(16, 16, bytes.into()).unwrap());
    let material = Arc::new(
        Material::textured(0xffffffff, std::array::from_fn(|_| Some(texture.clone()))).unwrap(),
    );
    let (volume, id) = Volume::new([1; 3])
        .unwrap()
        .with_material(material)
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], id).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [64; 2]).unwrap();
    for mode in [RenderMode::Flat, RenderMode::PathTraced] {
        let mut options = settings(ReconstructionMode::Off);
        options.mode = mode;
        options.ambient = 1.;
        options.samples = 4;
        options.bounces = 1;
        options.lighting.shadows.mode = if mode == RenderMode::Flat {
            ShadowMode::Soft
        } else {
            ShadowMode::Off
        };
        let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
        options.reconstruction.mode = ReconstructionMode::Spatial;
        options.reconstruction.spatial_passes = 3;
        let filtered = renderer.render_rgba(&volume, &camera, &options).unwrap();
        let mut whites = 0;
        for (a, b) in raw
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .zip(filtered.pixels.as_chunks::<4>().0.iter())
        {
            if a[3] == 255 && a[0] > 100 {
                whites += 1;
                assert!(
                    a[0].abs_diff(b[0]) <= 2,
                    "white checker darkened: {a:?} -> {b:?}"
                );
            }
            if a[3] == 255 && a[0] == 0 {
                assert_eq!(b[0], 0);
            }
        }
        assert!(whites > 100);
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn gradual_sun_changes_reuse_short_history_but_light_switches_do_not_ghost() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let volume = volume();
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    let mut options = settings(ReconstructionMode::Temporal);
    options.ambient = 0.;
    options.lighting.shadows.mode = ShadowMode::Off;
    renderer.render_rgba(&volume, &camera, &options).unwrap();
    for frame in 1u32..8 {
        options.time = fraction::Fraction::new(frame, 30u32);
        options.lighting.key_direction[0] += 0.002;
        let image = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert!(!image.stats.history_reset);
        assert!(image.stats.history_reactive);
        assert!((2..=4).contains(&image.stats.history_frames));
        assert_eq!(image.stats.upload_bytes, 0);
    }
    options.lighting.key_strength = 0.;
    let dark = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert!(dark.stats.history_reset);
    let center = (32 + 32 * 64) * 4;
    assert_eq!(&dark.pixels[center..center + 3], &[0; 3]);
    options.lighting.key_strength = 0.0001;
    renderer.render_rgba(&volume, &camera, &options).unwrap();
    options.lighting.key_strength = 0.;
    assert!(
        renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .stats
            .history_reset,
        "even a very dim light switching off must discard its history"
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn resize_mode_switches_and_budget_failures_do_not_retain_target_history() {
    let volume = volume();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut options = settings(ReconstructionMode::Temporal);
    let mut maximum = 0;
    for iteration in 0..12 {
        let side = if iteration % 2 == 0 { 32 } else { 64 };
        options.width = side;
        options.height = side;
        let camera = Camera::fit(View::Front, volume.dimensions(), [side; 2]).unwrap();
        let frame = renderer.render_rgba(&volume, &camera, &options).unwrap();
        assert!(frame.stats.history_reset);
        maximum = maximum.max(frame.stats.resident_bytes);
        if side == 32 {
            assert!(frame.stats.reconstruction_bytes < 100_000);
        }
    }
    assert!(maximum < 1_000_000);
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    options.reconstruction.mode = ReconstructionMode::Off;
    let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert_eq!(raw.stats.reconstruction_bytes, 0);
    let mut constrained = Renderer::headless(Limits {
        max_gpu_bytes: 100_000,
        ..Default::default()
    })
    .unwrap();
    options.reconstruction.mode = ReconstructionMode::Temporal;
    assert!(
        constrained
            .render_rgba(&volume, &camera, &options)
            .err()
            .unwrap()
            .contains("budget")
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn color_only_palette_rebuilds_keep_the_same_texture_mips_and_geometry() {
    let texture =
        Arc::new(sanim_voxel::Texture::new(16, 16, vec![255; 16 * 16 * 4].into()).unwrap());
    let build = |color| {
        let material = Arc::new(
            Material::textured(color, std::array::from_fn(|_| Some(texture.clone()))).unwrap(),
        );
        let (volume, id) = Volume::new([16; 3])
            .unwrap()
            .with_material(material)
            .unwrap();
        volume.fill_box([0; 3], [16; 3], id).unwrap()
    };
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let camera = Camera::fit(View::Front, [16; 3], [64; 2]).unwrap();
    let mut options = settings(ReconstructionMode::Spatial);
    options.ambient = 1.;
    let red = build(0xff0000ff);
    let first = renderer.render_rgba(&red, &camera, &options).unwrap();
    let blue = build(0x0000ffff);
    let changed = renderer.render_rgba(&blue, &camera, &options).unwrap();
    assert_eq!(
        changed.stats.texture_allocations, 0,
        "unchanged texture identity must retain every mip"
    );
    assert_eq!(changed.stats.uploaded_chunks, 0);
    assert_eq!(changed.stats.updated_pages, 0);
    assert_eq!(
        changed.stats.upload_bytes,
        2 * 20 * 4,
        "only the two material records should upload"
    );
    assert_eq!(first.stats.resident_bytes, changed.stats.resident_bytes);
    let center = (32 + 32 * 64) * 4;
    assert_eq!(&changed.pixels[center..center + 4], &[0, 0, 255, 255]);
    let held = renderer.render_rgba(&blue, &camera, &options).unwrap();
    assert_eq!(held.stats.upload_bytes, 0);
    assert_eq!(held.stats.texture_allocations, 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn deterministic_hard_shadows_are_not_spatially_blurred() {
    let volume = Volume::new([16; 3])
        .unwrap()
        .fill_color([0, 0, 0], [16, 2, 16], 0xffffffff)
        .unwrap()
        .fill_color([6, 2, 6], [9, 9, 9], 0xffffffff)
        .unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let camera = Camera::fit(View::Isometric, [16; 3], [64; 2]).unwrap();
    let mut options = settings(ReconstructionMode::Off);
    options.lighting.shadows.mode = ShadowMode::Hard;
    options.ambient = 0.;
    let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
    options.reconstruction.mode = ReconstructionMode::Spatial;
    options.reconstruction.spatial_passes = 3;
    let result = renderer.render_rgba(&volume, &camera, &options).unwrap();
    assert_eq!(
        raw.pixels, result.pixels,
        "deterministic visibility must preserve every hard-shadow edge"
    );
    assert_eq!(result.stats.reconstruction_passes, 0);
    assert_eq!(result.stats.reconstruction_bytes, 0);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn temporal_progressively_reduces_directional_lighting_variance() {
    let volume = Volume::new([1; 3])
        .unwrap()
        .fill_color([0; 3], [1; 3], 0xffffffff)
        .unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [64; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut options = settings(ReconstructionMode::Off);
    options.mode = RenderMode::PathTraced;
    options.bounces = 2;
    options.samples = 128;
    options.lights = Some(Arc::from([]));
    options.environment = Some(sanim_voxel::Environment {
        top: [1., 0., 0.],
        horizon: [0.2, 0.2, 0.2],
        ground: [0., 0., 1.],
        intensity: 1.,
        visible: false,
    });
    let reference = renderer.render_rgba(&volume, &camera, &options).unwrap();
    options.samples = 1;
    options.reconstruction = Reconstruction {
        mode: ReconstructionMode::Temporal,
        spatial_passes: 0,
        history_frames: 32,
        antialias_samples: 1,
    };
    let first = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let mut last = first.pixels.clone();
    for _ in 1..32 {
        last = renderer
            .render_rgba(&volume, &camera, &options)
            .unwrap()
            .pixels;
    }
    let error = |pixels: &[u8]| {
        (16..48)
            .flat_map(|y| {
                (16..48).flat_map(move |x| {
                    (0..3).map(move |channel| ((y * 64 + x) * 4 + channel) as usize)
                })
            })
            .map(|index| {
                let d = f64::from(pixels[index]) - f64::from(reference.pixels[index]);
                d * d
            })
            .sum::<f64>()
    };
    let before = error(&first.pixels);
    let after = error(&last);
    assert!(
        after < before * 0.7,
        "history must average independently sampled lighting: first={before}, accumulated={after}"
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn nonzero_near_black_albedo_survives_hdr_reconstruction() {
    let texture = Arc::new(sanim_voxel::Texture::new(1, 1, Arc::from([1u8, 1, 1, 255])).unwrap());
    let material = Arc::new(
        Material::textured(0x010101ff, std::array::from_fn(|_| Some(texture.clone()))).unwrap(),
    );
    let (volume, id) = Volume::new([1; 3])
        .unwrap()
        .with_material(material)
        .unwrap();
    let volume = volume.fill_box([0; 3], [1; 3], id).unwrap();
    let camera = Camera::fit(View::Front, [1; 3], [64; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut options = settings(ReconstructionMode::Off);
    options.mode = RenderMode::PathTraced;
    options.samples = 4;
    options.bounces = 1;
    options.lighting.key_strength = 1e10;
    options.lighting.key_direction = [0., 0., 1.];
    options.lighting.shadows.mode = ShadowMode::Off;
    let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let center = (32 + 32 * 64) * 4;
    assert!(
        raw.pixels[center] > 50,
        "HDR must make this tiny positive albedo visible"
    );
    for mode in [ReconstructionMode::Spatial, ReconstructionMode::Temporal] {
        options.reconstruction.mode = mode;
        let filtered = renderer.render_rgba(&volume, &camera, &options).unwrap();
        for channel in 0..3 {
            assert!(
                raw.pixels[center + channel].abs_diff(filtered.pixels[center + channel]) <= 3,
                "near-black channel lost: {:?} -> {:?}",
                &raw.pixels[center..center + 4],
                &filtered.pixels[center..center + 4]
            );
        }
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn zoomed_parallel_terraces_do_not_share_shadowed_lighting() {
    // One screen pixel spans several cells. The two front faces differ by only
    // one Z cell, yet the left face is shaded by a roof outside the camera ray.
    let volume = Volume::new([256, 128, 8])
        .unwrap()
        .fill_color([0, 0, 0], [128, 127, 1], 0xffffffff)
        .unwrap()
        .fill_color([128, 0, 1], [256, 127, 2], 0xffffffff)
        .unwrap()
        .fill_color([0, 127, 0], [128, 128, 8], 0xffffffff)
        .unwrap();
    let camera = Camera::fit(View::Front, volume.dimensions(), [64; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut options = settings(ReconstructionMode::Off);
    options.mode = RenderMode::PathTraced;
    options.samples = 1;
    options.bounces = 1;
    options.lighting.key_direction = [0., 1., 0.02];
    options.lighting.shadows.mode = ShadowMode::Hard;
    let raw = renderer.render_rgba(&volume, &camera, &options).unwrap();
    options.reconstruction.mode = ReconstructionMode::Spatial;
    options.reconstruction.spatial_passes = 3;
    let filtered = renderer.render_rgba(&volume, &camera, &options).unwrap();
    let mut compared = 0;
    for y in 34..41 {
        let left = (31 + y * 64) * 4;
        let right = (32 + y * 64) * 4;
        if raw.pixels[left] == 0 && raw.pixels[right] > 20 {
            compared += 1;
            for index in [left, right] {
                assert!(
                    raw.pixels[index].abs_diff(filtered.pixels[index]) <= 2,
                    "parallel-layer bleed at y={y}: {:?} -> {:?}",
                    &raw.pixels[index..index + 3],
                    &filtered.pixels[index..index + 3]
                );
            }
        }
    }
    assert!(
        compared >= 3,
        "scene must exercise adjacent independently lit planes; compared={compared}"
    );
}
