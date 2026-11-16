use fraction::Fraction;
use sanim_voxel::{
    camera::path::{Interpolation, Path},
    layout::Coordinate,
};
use std::sync::Arc;

fn points(count: usize) -> Arc<[[Coordinate; 3]]> {
    (0..count)
        .map(|index| {
            [
                Coordinate::Cells(Fraction::from(index)),
                Coordinate::Cells(Fraction::from(index % 7)),
                Coordinate::Cells(Fraction::from(index % 11)),
            ]
        })
        .collect()
}

#[test]
fn relative_coordinates_are_shared_with_layout_and_keep_signed_camera_offsets() {
    let coordinate = Coordinate::Percent(Fraction::from(50));
    assert_eq!(
        sanim_voxel::layout::coordinate(coordinate, 80).unwrap(),
        Fraction::from(40)
    );
    let negative = Coordinate::Percent(-Fraction::from(25));
    assert_eq!(
        sanim_voxel::camera::path::resolve_position(
            [negative, coordinate, Coordinate::Cells(3.into())],
            [80, 100, 120]
        )
        .unwrap(),
        [-20.0, 50.0, 3.0]
    );
    let path = Path::new(
        Arc::from([
            [Coordinate::Percent(0.into()); 3],
            [Coordinate::Percent(100.into()); 3],
        ]),
        Interpolation::Linear,
        false,
    )
    .unwrap();
    assert!(
        path.sample([80, 40, 20], 0.5, 0.0)
            .unwrap()
            .position
            .into_iter()
            .zip([40.0, 20.0, 10.0])
            .all(|(a, b)| (a - b).abs() < 1e-10)
    );
}

#[test]
fn open_and_closed_paths_hold_or_wrap_with_finite_transport_at_vertical_tangents() {
    let path = Path::new(points(12), Interpolation::Centripetal, true).unwrap();
    for progress in [-1.0, 0.0, 0.2, 0.5, 0.9, 1.0, 2.0] {
        let sample = path.sample([100; 3], progress, 2.0).unwrap();
        assert!(
            sample
                .position
                .into_iter()
                .chain(sample.target)
                .chain(sample.up)
                .all(f64::is_finite)
        );
        let direction =
            std::array::from_fn::<_, 3, _>(|axis| sample.target[axis] - sample.position[axis]);
        assert!(
            direction
                .iter()
                .zip(sample.up)
                .map(|(a, b)| a * b)
                .sum::<f64>()
                .abs()
                < 1e-8
        );
    }
    assert_eq!(
        path.sample([100; 3], 0.0, 2.0).unwrap().position,
        path.sample([100; 3], 1.0, 2.0).unwrap().position
    );
    let path = Path::new(points(12), Interpolation::Centripetal, false).unwrap();
    assert_eq!(
        path.sample([1; 3], -1.0, 0.0).unwrap().position,
        path.sample([1; 3], 0.0, 0.0).unwrap().position
    );
    assert_eq!(
        path.sample([1; 3], 2.0, 0.0).unwrap().position,
        path.sample([1; 3], 1.0, 0.0).unwrap().position
    );
}

#[test]
fn alternating_normalized_and_world_sampling_retains_exactly_two_compilations() {
    let path = Path::new(points(256), Interpolation::Centripetal, false).unwrap();
    let bytes = path.resident_bytes();
    for index in 0..1000 {
        let p = f64::from(index) / 1000.0;
        let a = path.sample([1; 3], p, 0.0).unwrap();
        let b = path.sample([176, 96, 176], p, 0.0).unwrap();
        assert_eq!(a.position, b.position);
        assert_eq!(bytes, path.resident_bytes());
    }
    // Sampling many world sizes replaces cache slots rather than retaining history.
    for size in 1..100 {
        path.sample([size; 3], 0.5, 0.0).unwrap();
    }
    assert_eq!(bytes, path.resident_bytes());
}

#[test]
#[ignore = "manual camera curve performance comparison"]
fn camera_curve_lookup_benchmark() {
    use std::{hint::black_box, time::Instant};
    let controls = points(256);
    let path = Path::new(controls.clone(), Interpolation::Centripetal, false).unwrap();
    for dims in [[1; 3], [176, 96, 176]] {
        path.sample(dims, 0.5, 0.0).unwrap();
    }
    let start = Instant::now();
    for index in 0..100_000 {
        black_box(
            path.sample(
                if index % 2 == 0 {
                    [1; 3]
                } else {
                    [176, 96, 176]
                },
                f64::from(index % 1000) / 1000.0,
                2.0,
            )
            .unwrap(),
        );
    }
    let cached = start.elapsed().as_secs_f64() / 100_000.0;
    let start = Instant::now();
    for _ in 0..100 {
        black_box(
            Path::new(controls.clone(), Interpolation::Centripetal, false)
                .unwrap()
                .sample([176, 96, 176], 0.5, 2.0)
                .unwrap(),
        );
    }
    let cold = start.elapsed().as_secs_f64() / 100.0;
    eprintln!(
        "256-point curve: cached alternating world/normalized {:.3}us/sample; rebuild {:.3}us/sample; {:.1}x; bounded charge {}B",
        cached * 1e6,
        cold * 1e6,
        cold / cached,
        path.resident_bytes()
    );
    assert!(cached < cold, "cache must improve actual repeated sampling");
}
