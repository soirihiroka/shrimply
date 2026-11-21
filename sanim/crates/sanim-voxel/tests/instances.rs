use sanim_voxel::{InstanceScene, MAX_BUCKET_CANDIDATES, SceneSource, Volume, math::raycast};
use std::sync::Arc;
fn asset(size: [u32; 3], color: u32) -> Arc<Volume> {
    Arc::new(
        Volume::new(size)
            .unwrap()
            .fill_color([0; 3], size, color)
            .unwrap(),
    )
}

#[test]
fn arbitrary_integer_origins_and_last_nonempty_precedence_match_flattening() {
    let red = Arc::new(
        asset([8; 3], 0xff0000ff)
            .fill_box([2; 3], [6; 3], 0)
            .unwrap(),
    );
    let blue = asset([4; 3], 0x3584e4ff);
    let scene = InstanceScene::new([24; 3], vec![(red, [1, 2, 3]), (blue, [5, 4, 3])]).unwrap();
    let flat = scene.flatten().unwrap();
    let source = SceneSource::Instances(Arc::new(scene.clone()));
    assert!(source.occupied_voxels().is_none());
    assert!(source.occupied_voxels_upper_bound() >= flat.occupied_voxels());
    for z in 0..24 {
        for y in 0..24 {
            for x in 0..24 {
                assert_eq!(
                    scene.cell([x, y, z]),
                    flat.get([x, y, z]).unwrap(),
                    "cell{x},{y},{z}"
                );
            }
        }
    }
    for x in 0..24 {
        for y in 0..24 {
            let origin = [f64::from(x) + 0.25, f64::from(y) + 0.5, 32.];
            assert_eq!(
                raycast(&scene, origin, [0., 0., -1.]).unwrap(),
                raycast(&flat, origin, [0., 0., -1.]).unwrap()
            );
        }
    }
}
#[test]
fn repeated_assets_share_payload_remaps_and_serialize_only_once() {
    let original = asset([4; 3], 0x33d17aff);
    let parts = (0..1000)
        .map(|i| {
            (
                original.clone(),
                [(i % 16) * 8, ((i / 16) % 4) * 8, (i / 64) * 8],
            )
        })
        .collect();
    let scene = InstanceScene::new([128, 32, 128], parts).unwrap();
    for instance in scene.instances() {
        assert!(Arc::ptr_eq(&original, &instance.volume));
        assert!(Arc::ptr_eq(
            &scene.instances()[0].materials,
            &instance.materials
        ));
    }
    assert!(scene.metadata_bytes() < 1024 * 1024);
    let encoded = serde_json::to_value(&scene).unwrap();
    assert_eq!(encoded["assets"].as_array().unwrap().len(), 1);
    let restored: InstanceScene = serde_json::from_value(encoded).unwrap();
    assert_eq!(scene, restored);
    assert!(Arc::ptr_eq(
        &restored.instances()[0].volume,
        &restored.instances()[999].volume
    ));
}
#[test]
fn translated_neighboring_chunks_do_not_duplicate_bucket_candidates() {
    let scene = InstanceScene::new([24; 3], vec![(asset([16; 3], 0xffffffff), [1; 3])]).unwrap();
    assert_eq!(scene.buckets().buckets.len(), 27);
    assert_eq!(scene.buckets().indices.len(), 27);
    assert!(scene.buckets().buckets.iter().all(|bucket| bucket.len == 1));
}
#[test]
fn bounded_candidates_and_containing_dimensions_fail_before_rendering() {
    let source = asset([1; 3], 0xffffffff);
    let error = InstanceScene::new(
        [8; 3],
        vec![(source.clone(), [0; 3]); MAX_BUCKET_CANDIDATES + 1],
    )
    .unwrap_err();
    assert!(error.contains("64 candidates"), "{error}");
    assert!(
        InstanceScene::new([8; 3], vec![(source.clone(), [u32::MAX; 3])])
            .unwrap_err()
            .contains("exceeds")
    );
    let scene = InstanceScene::new([8; 3], vec![(source, [0; 3])]).unwrap();
    let mut encoded = serde_json::to_value(&scene).unwrap();
    encoded["instances"][0]["asset"] = serde_json::json!(99);
    assert!(
        serde_json::from_value::<InstanceScene>(encoded)
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
}
#[test]
#[ignore = "manual unaligned dense-asset instance setup versus explicit flatten measurement"]
fn instancing_avoids_repeated_unaligned_voxel_copy_work() {
    use std::{hint::black_box, time::Instant};
    let mut source = Volume::new([32; 3])
        .unwrap()
        .fill_color([0; 3], [32; 3], 0x3584e4ff)
        .unwrap();
    source = source
        .fill_color([3, 0, 3], [19, 32, 19], 0xff7800ff)
        .unwrap();
    let source = Arc::new(source);
    let parts = (0..16)
        .map(|i| (source.clone(), [(i % 4) * 40 + 1, 1, (i / 4) * 40 + 1]))
        .collect::<Vec<_>>();
    let _warm = InstanceScene::new([168, 40, 168], parts.clone()).unwrap();
    let mut build = Vec::new();
    let mut flatten = Vec::new();
    let mut metadata = 0;
    for _ in 0..7 {
        let started = Instant::now();
        let scene = InstanceScene::new([168, 40, 168], parts.clone()).unwrap();
        build.push(started.elapsed());
        metadata = scene.metadata_bytes();
        let started = Instant::now();
        black_box(scene.flatten().unwrap());
        flatten.push(started.elapsed());
    }
    build.sort();
    flatten.sort();
    eprintln!(
        "16 unaligned reused 32^3 assets: instance setup {:?}, explicit flatten {:?}, metadata {metadata} bytes, retained unique asset {} bytes",
        build[3],
        flatten[3],
        source.resident_bytes()
    );
    assert!(
        build[3] < flatten[3],
        "instancing must actually avoid bulk copying work"
    );
}
