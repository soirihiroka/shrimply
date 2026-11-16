use serde_json::Value;
use std::{
    fs,
    process::{Command, Output},
};

const SOURCE: &str = "let grid=VoxelGrid(size=[8,8,8]).fill([0,0,0],[8,8,8],Color(\"orange3\"))\nlet scene=VoxelScene(volume=grid)\nscene.fade_in(duration=100ms)\nwait(1s)";

fn run(arguments: &[&str]) -> Output {
    run_source(SOURCE, arguments)
}

fn run_source(source_text: &str, arguments: &[&str]) -> Output {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("scene.sanim");
    fs::write(&source, source_text).unwrap();
    Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["voxel", arguments[0]])
        .arg(source)
        .args(&arguments[1..])
        .args([
            "--json", "--width", "64", "--height", "64", "--time", "200ms",
        ])
        .output()
        .unwrap()
}

#[test]
fn inspection_lists_multiple_cameras_without_initializing_a_gpu() {
    let result = run(&["inspect", "--views", "front,right,top,isometric"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["occupied_voxels"], 512);
    let views = report["views"].as_array().unwrap();
    assert_eq!(views.len(), 4);
    assert_eq!(views[0]["view"], "front");
    assert_eq!(views[2]["view"], "top");
    assert!(views.iter().all(|view| view.get("image").is_none()));
    assert_eq!(report["time"], "1/5s");
}

#[test]
fn picking_reports_a_voxel_face_in_the_named_view() {
    let result = run(&["pick", "--view", "front", "--pixel", "32,32"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["hit"]["position"][2], 7);
    assert_eq!(report["hit"]["normal"], serde_json::json!([0.0, 0.0, 1.0]));
    assert!(report["hit"]["distance"].as_f64().unwrap() > 0.);
}

#[test]
fn ambiguous_view_flags_and_missing_pick_coordinates_fail_cleanly() {
    let missing = run(&["pick"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--pixel"));
    let duplicate = run(&["inspect", "--view", "front", "--views", "right"]);
    assert_eq!(duplicate.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("specify one"));
}

#[test]
fn bounded_watch_counts_failed_render_attempts_and_reports_no_published_frame() {
    use std::{
        process::Stdio,
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("empty.sanim");
    fs::write(&source, "wait(1s)").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_sanim"))
        .args(["voxel", "watch"])
        .arg(source)
        .args(["--cycles", "1", "--output"])
        .arg(directory.path().join("preview.png"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("bounded voxel watch did not stop after a failed render attempt");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["event"], "failed");
    assert!(report["last_good_revision"].is_null());
    assert!(!directory.path().join("preview.png").exists());
}

#[test]
fn strict_framing_uses_the_same_normalized_margin_as_snapshot_assertions() {
    let zero = SOURCE.replace(
        "volume=grid",
        "volume=grid,check_framing=true,frame_margin=0%",
    );
    let output = run_source(&zero, &["inspect"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let strict = SOURCE.replace(
        "volume=grid",
        "volume=grid,check_framing=true,frame_margin=30%",
    );
    let snapshot = format!("{SOURCE}\nscene.assert_framed(margin=30%)");
    for source in [strict, snapshot] {
        let output = run_source(&source, &["inspect"]);
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("V003"),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn headless_inspection_reports_typed_lighting_and_ao_overrides() {
    let result = run(&[
        "inspect",
        "--shadows",
        "soft",
        "--shadow-strength",
        ".6",
        "--shadow-samples",
        "2",
        "--ao",
        "ray-traced",
        "--ao-strength",
        ".25",
        "--ao-distance",
        "6",
        "--ao-samples",
        "3",
        "--exposure",
        "-1",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(report["lighting"]["shadows"]["mode"], "soft");
    assert!((report["lighting"]["shadows"]["strength"].as_f64().unwrap() - 0.6).abs() < 1e-6);
    assert_eq!(report["lighting"]["shadows"]["samples"], 2);
    assert_eq!(report["ambient_occlusion"]["mode"], "ray-traced");
    assert_eq!(report["ambient_occlusion"]["strength"], 0.25);
    assert_eq!(report["ambient_occlusion"]["distance"], 6.);
    assert_eq!(report["ambient_occlusion"]["samples"], 3);
    assert_eq!(report["exposure_ev"], -1.);
    for arguments in [
        vec!["inspect", "--shadows", "invalid"],
        vec!["inspect", "--shadow-samples", "0"],
    ] {
        let output = run(&arguments);
        assert!(!output.status.success());
    }
}

#[test]
#[ignore = "requires GPU; compares actual survival water shadows with the CPU geometric oracle"]
fn survival_water_hard_shadows_match_cpu_visibility() {
    use sanim_core::{Fraction, Geometry, Number, Unit, Value};
    use sanim_lang::{Engine, FileSources};
    use sanim_voxel::math::SURFACE_OFFSET;
    use sanim_voxel_render::{
        AmbientOcclusion, AoMode, Limits, RenderSettings, Renderer, ShadowMode,
    };
    let directory = tempfile::tempdir().unwrap();
    let generator =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/voxel_pack.py");
    assert!(
        Command::new("uv")
            .args(["run", "--no-project", "python"])
            .arg(generator)
            .arg(directory.path())
            .status()
            .unwrap()
            .success()
    );
    // This regression isolates opaque visibility precision, independent of the water showcase's optics.
    let source = directory.path().join("survival.sanim");
    let text = fs::read_to_string(&source).unwrap();
    let text=text.lines().map(|line|if line.starts_with("let water = ") { "let water = VoxelMaterial(texture=read<VoxelTexture>(\"assets/minecraft/textures/block/water_still.png\"), transmission=0)" } else {line}).collect::<Vec<_>>().join("\n");
    fs::write(source, text).unwrap();
    let sources = FileSources::new(directory.path()).unwrap();
    let prepared = Engine::default()
        .prepare("survival.sanim", &sources, &Default::default())
        .unwrap();
    let Value::VoxelMaterial(water) = &prepared.bindings["water"] else {
        panic!("water material");
    };
    let mut options = RenderSettings {
        width: 1280,
        height: 720,
        ambient: 0.,
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        ambient_occlusion: AmbientOcclusion {
            mode: AoMode::Off,
            ..Default::default()
        },
        ..Default::default()
    };
    options.lighting.shadows.mode = ShadowMode::Hard;
    let viewport = [options.width, options.height];
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let selected = std::env::var("SANIM_VOXEL_ORACLE_TIME").ok().map(|text| {
        let time = Number::parse(&text).unwrap();
        assert_eq!(time.unit, Unit::Duration);
        time.value
    });
    for time in [
        Fraction::from(1),
        Fraction::from(2),
        Fraction::from(4),
        Fraction::from(6),
        Fraction::new(43u64, 5u64),
    ] {
        if selected.is_some_and(|selected| selected != time) {
            continue;
        }
        let elements = prepared.animation.sample(time).unwrap();
        let scene = elements
            .iter()
            .find_map(|element| match element.geometry.as_ref() {
                Geometry::Voxel(scene) => Some(scene),
                _ => None,
            })
            .unwrap();
        let camera = scene.resolve_camera(viewport).unwrap();
        let frame = match &scene.source {
            sanim_voxel::SceneSource::Volume(volume) => {
                renderer.render_rgba(volume, &camera, &options)
            }
            sanim_voxel::SceneSource::Instances(instances) => {
                renderer.render_instances_rgba(instances, &camera, &options)
            }
        }
        .unwrap();
        let light = options.lighting.key_direction.map(f64::from);
        let mut candidates = 0;
        let mut compared = 0;
        let mut blocked = 0;
        let mut mismatches = Vec::new();
        for y in (0..options.height).step_by(2) {
            for x in (0..options.width).step_by(2) {
                let ray = camera.ray_for_pixel(viewport, [x, y]).unwrap();
                let Some(hit) = scene.raycast(ray.origin, ray.direction).unwrap() else {
                    continue;
                };
                if hit.normal != [0.0, 1.0, 0.0]
                    || scene.material(hit.material).unwrap().as_ref() != water.as_ref()
                {
                    continue;
                }
                candidates += 1;
                let norm = ray
                    .direction
                    .iter()
                    .map(|axis| axis * axis)
                    .sum::<f64>()
                    .sqrt();
                let point = std::array::from_fn(|axis| {
                    ray.origin[axis]
                        + ray.direction[axis] / norm * hit.distance
                        + hit.normal[axis] * SURFACE_OFFSET
                });
                let expected = scene.raycast(point, light).unwrap().is_some();
                // Discard geometric shadow boundaries; tiny CPU/GPU rounding differences are legitimate there.
                let mut stable = true;
                for dx in [-0.01, 0.01] {
                    for dz in [-0.01, 0.01] {
                        let probe = [point[0] + dx, point[1], point[2] + dz];
                        stable &= scene.raycast(probe, light).unwrap().is_some() == expected;
                    }
                }
                if !stable {
                    continue;
                }
                compared += 1;
                blocked += usize::from(expected);
                let index = ((y * options.width + x) * 4) as usize;
                let rgb = &frame.pixels[index..index + 3];
                let observed = rgb.iter().all(|channel| *channel <= 2);
                let visible_water = observed || rgb[2].saturating_sub(rgb[0]) > 16;
                if observed != expected || !visible_water {
                    mismatches.push(([x, y], hit.position, expected, rgb.to_vec()));
                }
            }
        }
        eprintln!(
            "survival water at {time}s: {compared}/{candidates} stable water pixels ({} boundary-uncertain excluded), {blocked} CPU-occluded, {} visibility mismatches; first {:?}",
            candidates - compared,
            mismatches.len(),
            &mismatches[..mismatches.len().min(10)]
        );
        assert!(
            compared > 100 && blocked > 10 && compared - blocked > 10,
            "test must include both shaded and lit river water"
        );
        assert!(
            mismatches.is_empty(),
            "GPU hard-shadow visibility disagrees with the CPU oracle"
        );
    }
}

#[test]
fn reconstruction_controls_are_typed_and_validated_without_a_gpu() {
    let result = run(&[
        "inspect",
        "--reconstruction",
        "temporal",
        "--spatial-passes",
        "2",
        "--history-frames",
        "12",
        "--antialias",
        "4",
    ]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(
        report["reconstruction"],
        serde_json::json!({
            "mode":"temporal","spatial_passes":2,"history_frames":12,"antialias_samples":4
        })
    );
    for (flag, value, reason) in [
        ("--reconstruction", "blur", "reconstruction"),
        ("--spatial-passes", "4", "spatial"),
        ("--history-frames", "0", "history"),
        ("--history-frames", "33", "history"),
        ("--antialias", "0", "antialias"),
        ("--antialias", "5", "antialias"),
    ] {
        let invalid = run(&["inspect", flag, value]);
        assert!(!invalid.status.success(), "{flag}={value}");
        assert!(
            String::from_utf8_lossy(&invalid.stderr)
                .to_lowercase()
                .contains(reason),
            "{}",
            String::from_utf8_lossy(&invalid.stderr)
        );
    }
}
