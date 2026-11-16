//! Opt-in GPU profiling of the actual generated, animated showcase; no test-only scene substitute.
use sanim_core::{Fraction, Geometry};
use sanim_voxel::{AoMode, ShadowMode};
use sanim_voxel_render::{
    AmbientOcclusion, Frame, Limits, Reconstruction, ReconstructionMode, RenderMode,
    RenderSettings, Renderer,
};
use std::time::Instant;
#[path = "support/voxel.rs"]
mod fixture;

#[test]
#[ignore = "manual actual-valley GPU performance investigation"]
fn actual_valley_same_process_gpu_benchmark() {
    let (_temporary, prepared) = fixture::valley();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut initialized = false;
    let preview_only = std::env::var_os("SANIM_VOXEL_PROFILE_PREVIEW").is_some();
    let sizes = if preview_only {
        vec![[1280, 720]]
    } else if std::env::var_os("SANIM_VOXEL_PROFILE_SMALL").is_some() {
        vec![[64, 36]]
    } else {
        vec![[64, 36], [256, 144]]
    };
    for size in sizes {
        for seconds in [2, 8] {
            let time = Fraction::from(seconds);
            let elements = prepared.animation.sample(time).unwrap();
            let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
                panic!("showcase must contain the native VoxelScene");
            };
            let camera = scene.resolve_camera(size).unwrap();
            let mut options = RenderSettings {
                width: size[0],
                height: size[1],
                mode: if preview_only {
                    RenderMode::Flat
                } else {
                    RenderMode::PathTraced
                },
                lights: scene.lights.clone(),
                environment: scene.environment,
                time,
                reconstruction: Reconstruction {
                    mode: ReconstructionMode::Off,
                    antialias_samples: 1,
                    ..Default::default()
                },
                ambient_occlusion: AmbientOcclusion {
                    mode: AoMode::Off,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut draw = |options: &RenderSettings| -> Frame {
                let frame = match &scene.source {
                    sanim_voxel::SceneSource::Volume(volume) => {
                        renderer.render_rgba(volume, &camera, options)
                    }
                    sanim_voxel::SceneSource::Instances(instances) => {
                        renderer.render_instances_rgba(instances, &camera, options)
                    }
                }
                .unwrap();
                if initialized {
                    assert_eq!(
                        frame.stats.uploaded_chunks, 0,
                        "camera, light, and render-quality changes must retain asset geometry"
                    );
                    assert_eq!(frame.stats.upload_bytes, 0);
                }
                initialized = true;
                frame
            };
            // Compile/allocate once outside timings, then compare actual GPU export
            // in this same process, with exactly the same immutable scene source.
            draw(&options);
            options.mode = RenderMode::Flat;
            for shadows in [ShadowMode::Off, ShadowMode::Soft] {
                options.lighting.shadows.mode = shadows;
                options.lighting.shadows.samples = 4;
                draw(&options);
                let mut timings = Vec::new();
                let mut resident = 0;
                for _ in 0..3 {
                    let start = Instant::now();
                    let frame = draw(&options);
                    timings.push(start.elapsed().as_micros());
                    resident = frame.stats.resident_bytes;
                }
                timings.sort_unstable();
                eprintln!(
                    "actual-valley size={}x{} time={seconds}s mode=flat shadows={shadows} median_us={} gpu_bytes={resident}",
                    size[0], size[1], timings[1]
                );
            }
            if preview_only {
                options.reconstruction = Reconstruction::default();
                options.lighting.shadows.mode = ShadowMode::Off;
                draw(&options);
                let mut timings = Vec::new();
                let mut resident = 0;
                for _ in 0..3 {
                    let start = Instant::now();
                    let frame = draw(&options);
                    timings.push(start.elapsed().as_micros());
                    resident = frame.stats.resident_bytes;
                }
                timings.sort_unstable();
                eprintln!(
                    "actual-valley default-preview time={seconds}s median_us={} gpu_bytes={resident}",
                    timings[1]
                );
                options.lighting.shadows.mode = ShadowMode::Soft;
                options.lighting.shadows.samples = 1;
                for (mode, passes, antialias) in [
                    (ReconstructionMode::Spatial, 2, 2),
                    (ReconstructionMode::Spatial, 3, 4),
                    (ReconstructionMode::Temporal, 2, 4),
                ] {
                    options.reconstruction = Reconstruction {
                        mode,
                        spatial_passes: passes,
                        antialias_samples: antialias,
                        ..Default::default()
                    };
                    draw(&options);
                    let mut timings = Vec::new();
                    let mut resident = 0;
                    for _ in 0..3 {
                        let start = Instant::now();
                        let frame = draw(&options);
                        timings.push(start.elapsed().as_micros());
                        resident = frame.stats.resident_bytes;
                    }
                    timings.sort_unstable();
                    eprintln!(
                        "actual-valley preview reconstruction={mode} passes={passes} aa={antialias} shadows=soft1 time={seconds}s median_us={} gpu_bytes={resident}",
                        timings[1]
                    );
                }
                continue;
            }
            options.mode = RenderMode::PathTraced;
            options.lighting.shadows.samples = 1;
            for samples in [4, 32, 128] {
                for bounces in [1, 4] {
                    for shadows in [ShadowMode::Off, ShadowMode::Soft] {
                        options.samples = samples;
                        options.bounces = bounces;
                        options.lighting.shadows.mode = shadows;
                        let start = Instant::now();
                        let frame = draw(&options);
                        eprintln!(
                            "actual-valley size={}x{} time={seconds}s spp={samples} bounces={bounces} shadows={shadows} us={} gpu_bytes={}",
                            size[0],
                            size[1],
                            start.elapsed().as_micros(),
                            frame.stats.resident_bytes
                        );
                    }
                }
            }
        }
    }
}
