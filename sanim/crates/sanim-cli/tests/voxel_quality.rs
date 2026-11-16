//! Compare reconstruction against converged lighting on the actual example.
use sanim_core::{Fraction, Geometry};
use sanim_voxel_render::{
    Frame, Limits, Reconstruction, ReconstructionMode, RenderMode, RenderSettings, Renderer,
};
use std::time::{Duration, Instant};
#[path = "support/voxel.rs"]
mod fixture;

fn rmse(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    let squared = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .flat_map(|(a, b)| a[..3].iter().zip(&b[..3]))
        .map(|(a, b)| {
            let error = f64::from(*a) - f64::from(*b);
            error * error
        })
        .sum::<f64>();
    (squared / (a.len() / 4 * 3) as f64).sqrt()
}

fn measure(mut draw: impl FnMut() -> Frame) -> (Frame, Duration) {
    let mut frame = draw();
    let mut times = [Duration::ZERO; 3];
    for time in &mut times {
        let start = Instant::now();
        frame = draw();
        *time = start.elapsed();
    }
    times.sort_unstable();
    (frame, times[1])
}

#[test]
#[ignore = "manual actual-valley reconstruction quality/performance benchmark"]
fn actual_valley_reconstruction_quality_benchmark() {
    let (_temporary, prepared) = fixture::valley();
    let time = Fraction::from(3);
    let elements = prepared.animation.sample(time).unwrap();
    let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
        panic!("voxel scene")
    };
    let size = [320, 180];
    let camera = scene.resolve_camera(size).unwrap();
    let mut renderer = Renderer::headless(Limits::default()).unwrap();
    let mut options = RenderSettings {
        width: size[0],
        height: size[1],
        time,
        mode: RenderMode::PathTraced,
        samples: 128,
        bounces: 4,
        lights: scene.lights.clone(),
        environment: scene.environment,
        reconstruction: Reconstruction {
            mode: ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut draw = |options: &RenderSettings| -> Frame {
        match &scene.source {
            sanim_voxel::SceneSource::Volume(volume) => {
                renderer.render_rgba(volume, &camera, options)
            }
            sanim_voxel::SceneSource::Instances(instances) => {
                renderer.render_instances_rgba(instances, &camera, options)
            }
        }
        .unwrap()
    };
    let reference = draw(&options);
    for samples in [4, 16] {
        options.samples = samples;
        options.reconstruction = Reconstruction {
            mode: ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        };
        let (raw, raw_time) = measure(|| draw(&options));
        let before = rmse(&raw.pixels, &reference.pixels);
        for passes in [2, 3] {
            options.reconstruction = Reconstruction {
                mode: ReconstructionMode::Spatial,
                spatial_passes: passes,
                antialias_samples: 4,
                ..Default::default()
            };
            let (filtered, filtered_time) = measure(|| draw(&options));
            assert_eq!(filtered.stats.uploaded_chunks, 0);
            assert_eq!(filtered.stats.upload_bytes, 0);
            let after = rmse(&filtered.pixels, &reference.pixels);
            eprintln!(
                "actual-valley {}x{} reference128spp samples={samples} passes={passes} raw_rmse={before:.5} filtered_rmse={after:.5} raw_us={} filtered_us={} gpu_bytes={}",
                size[0],
                size[1],
                raw_time.as_micros(),
                filtered_time.as_micros(),
                filtered.stats.resident_bytes
            );
            assert!(
                after < before,
                "reconstruction must improve actual-scene error, not just blur a synthetic patch: raw={before}, filtered={after}"
            );
        }
    }
    options.samples = 4;
    options.reconstruction = Reconstruction {
        mode: ReconstructionMode::Temporal,
        spatial_passes: 2,
        history_frames: 16,
        antialias_samples: 4,
    };
    draw(&options);
    let start = Instant::now();
    let mut temporal = draw(&options);
    for _ in 2..16 {
        temporal = draw(&options);
    }
    let temporal_time = start.elapsed();
    let error = rmse(&temporal.pixels, &reference.pixels);
    eprintln!(
        "actual-valley temporal4spp history_frames=16 measured_frames=15 rmse={error:.5} total_us={} gpu_bytes={}",
        temporal_time.as_micros(),
        temporal.stats.resident_bytes
    );
    assert_eq!(temporal.stats.upload_bytes, 0);
    assert_eq!(temporal.stats.history_frames, 16);
}
