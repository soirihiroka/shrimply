use sanim_voxel::{
    AmbientOcclusion, AoMode, InstanceScene, Material, MaterialGeometry, OpticalProperties,
    Texture, Volume,
};
use sanim_voxel_render::{Camera, Limits, RenderSettings, Renderer, View};
use std::sync::Arc;

fn options() -> RenderSettings {
    RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 65,
        height: 65,
        ambient: 1.,
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::Off,
            ..Default::default()
        },
        ..Default::default()
    }
}
fn asset(color: u32) -> Arc<Volume> {
    let volume = Volume::new([9, 7, 11])
        .unwrap()
        .fill_color([0; 3], [9, 7, 11], color)
        .unwrap()
        .fill_box([4, 0, 0], [5, 7, 11], 0)
        .unwrap();
    Arc::new(volume)
}
fn flatten(dimensions: [u32; 3], parts: &[(Arc<Volume>, [u32; 3])]) -> Volume {
    parts.iter().fold(
        Volume::new(dimensions).unwrap(),
        |volume, (child, origin)| volume.place(child, origin.map(|axis| axis as i32)).unwrap(),
    )
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn arbitrary_instance_origins_overlap_and_empty_cells_match_the_flattened_oracle_in_all_views() {
    let a = asset(0xff0000ff);
    let b = asset(0x0000ffff);
    let parts = vec![(a.clone(), [3, 2, 5]), (b, [8, 4, 9]), (a, [19, 1, 18])];
    let scene = InstanceScene::new([32; 3], parts.clone()).unwrap();
    let flattened = flatten([32; 3], &parts);
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    for view in View::ALL {
        let camera = Camera::fit(view, [32; 3], [65; 2]).unwrap();
        let expected = renderer
            .render_rgba(&flattened, &camera, &options())
            .unwrap();
        let actual = renderer
            .render_instances_rgba(&scene, &camera, &options())
            .unwrap();
        assert!(
            actual.pixels == expected.pixels,
            "arbitrary translated occupancy differs from flattened oracle in {view}"
        );
        assert!(actual.stats.uploaded_chunks <= flattened.chunk_count());
    }
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn repeated_assets_share_gpu_geometry_and_placement_edits_do_not_upload_it() {
    let asset = asset(0x4a90d9ff);
    let scene = |count: u32, shift: u32| {
        InstanceScene::new(
            [64; 3],
            (0..count)
                .map(|index| (asset.clone(), [1 + shift + index * 12, 1, 2]))
                .collect(),
        )
        .unwrap()
    };
    let camera = Camera::fit(View::Isometric, [64; 3], [65; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let first = renderer
        .render_instances_rgba(&scene(1, 0), &camera, &options())
        .unwrap();
    let dense = asset
        .chunks()
        .filter(|(_, chunk)| chunk.uniform().is_none())
        .count();
    assert_eq!(first.stats.uploaded_chunks, dense);
    let repeated = renderer
        .render_instances_rgba(&scene(5, 0), &camera, &options())
        .unwrap();
    assert_eq!(
        repeated.stats.uploaded_chunks, 0,
        "adding arbitrarily placed copies must not upload asset cells again"
    );
    let mut maximum = repeated.stats.resident_bytes;
    for shift in 0..32 {
        let current = scene(5, shift % 3);
        let frame = renderer
            .render_instances_rgba(&current, &camera, &options())
            .unwrap();
        assert_eq!(frame.stats.uploaded_chunks, 0);
        assert_eq!(frame.stats.buffer_allocations, 0);
        maximum = maximum.max(frame.stats.resident_bytes);
        let held = renderer
            .render_instances_rgba(&current, &camera, &options())
            .unwrap();
        assert_eq!(held.stats.upload_bytes, 0);
    }
    assert!(
        maximum <= repeated.stats.resident_bytes + 4096,
        "placement metadata must remain bounded, not retain a history"
    );
    let reduced = renderer
        .render_instances_rgba(&scene(1, 0), &camera, &options())
        .unwrap();
    assert_eq!(reduced.stats.uploaded_chunks, 0);
    assert!(reduced.stats.resident_bytes <= maximum);
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn rebuilding_only_the_palette_reuses_exact_geometry_without_retaining_old_assets() {
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let camera = Camera::fit(View::Front, [16; 3], [65; 2]).unwrap();
    let mut previous = None;
    let mut resident = None;
    for step in 0..32 {
        let color = if step % 2 == 0 {
            0xff0000ff
        } else {
            0x0000ffff
        };
        let source = asset(color);
        let weak = Arc::downgrade(&source);
        let scene = InstanceScene::new([16; 3], vec![(source, [3, 2, 4])]).unwrap();
        let frame = renderer
            .render_instances_rgba(&scene, &camera, &options())
            .unwrap();
        if step != 0 {
            assert_eq!(frame.stats.uploaded_chunks, 0);
            assert_eq!(
                frame.stats.buffer_allocations, 1,
                "only the material palette allocation changes"
            );
        }
        if let Some(previous) = previous {
            assert!(
                std::sync::Weak::<Volume>::upgrade(&previous).is_none(),
                "old source asset allocations must be released after a palette edit"
            );
        }
        if let Some(resident) = resident {
            assert_eq!(frame.stats.resident_bytes, resident);
        }
        assert!(frame.pixels.chunks_exact(4).any(|pixel| if step % 2 == 0 {
            pixel[0] > 200 && pixel[2] < 20
        } else {
            pixel[2] > 200 && pixel[0] < 20
        }));
        resident = Some(frame.stats.resident_bytes);
        previous = Some(weak);
    }
    renderer.release_volume().unwrap();
    assert!(previous.unwrap().upgrade().is_none());
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn instanced_foliage_textures_and_transmission_use_the_same_leaf_renderer() {
    let (glass, glass_id) = Volume::new([8; 3])
        .unwrap()
        .with_material(Arc::new(
            Material::solid(0xffffffff)
                .unwrap()
                .with_optics(OpticalProperties {
                    transmission: 1.,
                    ior: 1.333,
                    attenuation_color: [0.4, 0.8, 1.],
                    attenuation_distance: 8.,
                    ..Default::default()
                })
                .unwrap(),
        ))
        .unwrap();
    let glass = Arc::new(glass.fill_box([0; 3], [8; 3], glass_id).unwrap());
    let texture = Arc::new(
        Texture::new(
            2,
            2,
            Arc::from([
                255, 255, 255, 255, 255, 255, 255, 0, 255, 255, 255, 0, 255, 255, 255, 255,
            ]),
        )
        .unwrap(),
    );
    let material = Material::textured(0x33d17aff, std::array::from_fn(|_| Some(texture.clone())))
        .unwrap()
        .with_geometry(MaterialGeometry::Cross)
        .unwrap();
    let (plant, plant_id) = Volume::new([2, 3, 2])
        .unwrap()
        .with_material(Arc::new(material))
        .unwrap();
    let plant = Arc::new(plant.fill_box([0; 3], [2, 3, 2], plant_id).unwrap());
    let parts = vec![(glass, [3, 1, 5]), (plant, [12, 1, 7])];
    let scene = InstanceScene::new([16; 3], parts.clone()).unwrap();
    let flat = flatten([16; 3], &parts);
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    for view in [View::Front, View::Top, View::Isometric] {
        let camera = Camera::fit(view, [16; 3], [65; 2]).unwrap();
        let expected = renderer.render_rgba(&flat, &camera, &options()).unwrap();
        let actual = renderer
            .render_instances_rgba(&scene, &camera, &options())
            .unwrap();
        assert!(
            actual
                .pixels
                .iter()
                .zip(&expected.pixels)
                .all(|(a, b)| (i32::from(*a) - i32::from(*b)).abs() <= 1),
            "shared leaf renderer mismatch in {view}"
        );
    }
}

#[test]
#[ignore = "GPU instancing performance benchmark"]
fn instance_benchmark() {
    use std::time::Instant;
    let asset = asset(0x4a90d9ff);
    let ground = Arc::new(
        Volume::new([128, 8, 128])
            .unwrap()
            .fill_color([0; 3], [128, 8, 128], 0xb0a080ff)
            .unwrap(),
    );
    let parts = |shift| {
        std::iter::once((ground.clone(), [0; 3]))
            .chain((0..64).map(|index| {
                (
                    asset.clone(),
                    [1 + (index % 8) * 15 + shift, 8, 1 + (index / 8) * 15],
                )
            }))
            .collect::<Vec<_>>()
    };
    let start = Instant::now();
    let source = InstanceScene::new([128, 32, 128], parts(0)).unwrap();
    let instance_build = start.elapsed().as_micros();
    let positions = parts(0)
        .into_iter()
        .skip(1)
        .map(|(_, p)| p.map(|v| v as i32))
        .collect::<Vec<_>>();
    let start = Instant::now();
    let flattened = Volume::new([128, 32, 128])
        .unwrap()
        .place(&ground, [0; 3])
        .unwrap()
        .place_many(&asset, &positions)
        .unwrap();
    let flatten_build = start.elapsed().as_micros();
    println!(
        "copies=64 instance_build_us={instance_build} optimized_flatten_build_us={flatten_build} instance_host_bytes={} flattened_host_bytes={}",
        source.resident_bytes(),
        flattened.resident_bytes()
    );
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let camera = Camera::fit(View::Isometric, [128, 32, 128], [640, 360]).unwrap();
    let options = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 640,
        height: 360,
        ..options()
    };
    let flat_first = renderer.render_rgba(&flattened, &camera, &options).unwrap();
    let warm = Instant::now();
    while warm.elapsed().as_millis() < 500 {
        renderer.render_rgba(&flattened, &camera, &options).unwrap();
    }
    let mut flat_times = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        renderer.render_rgba(&flattened, &camera, &options).unwrap();
        flat_times.push(start.elapsed().as_micros());
    }
    let first = renderer
        .render_instances_rgba(&source, &camera, &options)
        .unwrap();
    let warm = Instant::now();
    while warm.elapsed().as_millis() < 500 {
        renderer
            .render_instances_rgba(&source, &camera, &options)
            .unwrap();
    }
    let mut times = Vec::new();
    for _ in 0..9 {
        let start = Instant::now();
        let frame = renderer
            .render_instances_rgba(&source, &camera, &options)
            .unwrap();
        times.push(start.elapsed().as_micros());
        assert_eq!(frame.stats.upload_bytes, 0);
    }
    flat_times.sort_unstable();
    times.sort_unstable();
    println!(
        "flat_warm_median_us={} instance_warm_median_us={} flat_uploaded_chunks={} instance_uploaded_chunks={} flat_gpu_bytes={} instance_gpu_bytes={}",
        flat_times[4],
        times[4],
        flat_first.stats.uploaded_chunks,
        first.stats.uploaded_chunks,
        flat_first.stats.resident_bytes,
        first.stats.resident_bytes
    );
    let mut builds = Vec::new();
    let mut patches = Vec::new();
    for shift in 0..18 {
        let start = Instant::now();
        let source = InstanceScene::new([128, 32, 128], parts(shift % 3)).unwrap();
        builds.push(start.elapsed().as_micros());
        let start = Instant::now();
        let frame = renderer
            .render_instances_rgba(&source, &camera, &options)
            .unwrap();
        patches.push(start.elapsed().as_micros());
        assert_eq!(frame.stats.uploaded_chunks, 0);
    }
    builds.sort_unstable();
    patches.sort_unstable();
    println!(
        "instance_patch_build_median_us={} instance_patch_export_median_us={}",
        builds[9], patches[9]
    );
    let sparse_camera = Camera::fit(View::Front, [512; 3], [640, 360]).unwrap();
    let sparse = |x| InstanceScene::new([512; 3], vec![(asset.clone(), [x, 100, 100])]).unwrap();
    renderer
        .render_instances_rgba(&sparse(100), &sparse_camera, &options)
        .unwrap();
    let mut sparse_times = Vec::new();
    let mut maximum_upload = 0;
    let mut maximum_pages = 0;
    for x in [101, 108, 100, 101, 108, 100, 101, 108, 100] {
        let current = sparse(x);
        let start = Instant::now();
        let frame = renderer
            .render_instances_rgba(&current, &sparse_camera, &options)
            .unwrap();
        sparse_times.push(start.elapsed().as_micros());
        maximum_upload = maximum_upload.max(frame.stats.upload_bytes);
        maximum_pages = maximum_pages.max(frame.stats.updated_pages);
        assert_eq!(frame.stats.uploaded_chunks, 0);
        assert!(frame.stats.upload_bytes < 1024);
    }
    sparse_times.sort_unstable();
    println!(
        "sparse_world512_pose_export_median_us={} max_upload_bytes={maximum_upload} max_updated_pages={maximum_pages} full_world_page_table_bytes={}",
        sparse_times[4],
        (512u64 / 8).pow(3) * 4
    );
}

#[test]
#[ignore = "requires a Vulkan or Metal adapter"]
fn moving_a_small_asset_in_a_large_sparse_world_updates_only_affected_words() {
    let asset = asset(0x4a90d9ff);
    let make = |x| InstanceScene::new([512; 3], vec![(asset.clone(), [x, 100, 100])]).unwrap();
    let camera = Camera::fit(View::Front, [512; 3], [65; 2]).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let first = renderer
        .render_instances_rgba(&make(100), &camera, &options())
        .unwrap();
    for x in [101, 108, 100] {
        let frame = renderer
            .render_instances_rgba(&make(x), &camera, &options())
            .unwrap();
        assert_eq!(frame.stats.uploaded_chunks, 0);
        assert!(
            frame.stats.updated_pages < 32,
            "only affected sparse world pages need updates: {:?}",
            frame.stats
        );
        assert!(
            frame.stats.upload_bytes < 1024,
            "a small placement edit must not resend a1MiB world page table: {:?}",
            frame.stats
        );
        assert_eq!(frame.stats.resident_bytes, first.stats.resident_bytes);
    }
}
