use sanim_voxel::{
    Environment, Light, LightKind, Lighting, MAX_LIGHTS, PointLight, ShadowMode, SunLight,
    layout::Coordinate, scene_light_parameters,
};

#[test]
fn sun_preserves_existing_irradiance_and_basis_for_each_shadow_mode() {
    let sun = Light::Sun(SunLight::default());
    for mode in [ShadowMode::Off, ShadowMode::Hard, ShadowMode::Soft] {
        let mut legacy = Lighting::default();
        legacy.shadows.mode = mode;
        let legacy = legacy.parameters().unwrap();
        let current = sun.parameters([96, 52, 96], mode).unwrap();
        assert_eq!(legacy.direction, current.position_or_direction);
        assert_eq!(legacy.key_radiance, current.radiance);
        assert_eq!(legacy.cos_radius, current.cos_radius);
    }
}
#[test]
fn relative_points_resize_without_changing_the_declarative_recipe() {
    let light = Light::Point(PointLight {
        position: [
            Coordinate::Percent(50.into()),
            Coordinate::Cells((-2).into()),
            Coordinate::Percent(125.into()),
        ],
        intensity: 200.,
        ..Default::default()
    });
    let first = light.parameters([20, 10, 40], ShadowMode::Soft).unwrap();
    let second = light.parameters([40, 20, 80], ShadowMode::Soft).unwrap();
    assert_eq!(first.position_or_direction, [10., -2., 50.]);
    assert_eq!(second.position_or_direction, [20., -2., 100.]);
    assert_eq!(first.kind, LightKind::Point);
    assert_eq!(first.radiance, [200.; 3]);
}
#[test]
fn light_distribution_is_bounded_unbiased_and_robust_to_large_dynamic_ranges() {
    let lights = [
        Light::Sun(SunLight {
            intensity: 0.,
            ..Default::default()
        }),
        Light::Point(PointLight {
            intensity: 1e-12,
            ..Default::default()
        }),
        Light::Sun(SunLight {
            intensity: 1e12,
            ..Default::default()
        }),
    ];
    let parameters = scene_light_parameters(&lights, [96; 3], ShadowMode::Soft).unwrap();
    assert_eq!(parameters.len(), 2);
    assert!(parameters.iter().all(|light| light.pdf >= 0.25));
    assert_eq!(parameters.last().unwrap().cdf, 1.);
    assert!((parameters.iter().map(|light| light.pdf).sum::<f32>() - 1.).abs() < 1e-6);
    let estimate = parameters
        .iter()
        .map(|light| f64::from(light.pdf) * f64::from(light.radiance[0]) / f64::from(light.pdf))
        .sum::<f64>();
    let expected = parameters
        .iter()
        .map(|light| f64::from(light.radiance[0]))
        .sum::<f64>();
    assert_eq!(estimate, expected);
    assert!(
        scene_light_parameters(&vec![lights[0]; MAX_LIGHTS + 1], [96; 3], ShadowMode::Soft)
            .unwrap_err()
            .contains("maximum")
    );
    assert!(
        scene_light_parameters(&[], [96; 3], ShadowMode::Soft)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn invalid_light_fields_and_environment_radiance_fail_clearly() {
    assert!(
        Light::Sun(SunLight {
            direction: [0.; 3],
            ..Default::default()
        })
        .validate()
        .unwrap_err()
        .contains("nonzero")
    );
    assert!(
        Light::Point(PointLight {
            radius: -1.,
            ..Default::default()
        })
        .validate()
        .unwrap_err()
        .contains("radius")
    );
    assert!(
        Light::Point(PointLight {
            intensity: f32::INFINITY,
            ..Default::default()
        })
        .validate()
        .is_err()
    );
    let environment = Environment {
        intensity: 0.001,
        visible: false,
        ..Default::default()
    };
    let parameters = environment.parameters().unwrap();
    assert!(!parameters.visible);
    assert!(
        parameters
            .top
            .iter()
            .all(|value| *value > 0. && *value < 0.001)
    );
    assert!(
        Environment {
            intensity: -1.,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
#[test]
#[ignore = "manual 32-light setup/CDF preparation measurement"]
fn scene_light_setup_benchmark() {
    use std::{hint::black_box, time::Instant};
    let lights = vec![Light::Point(PointLight::default()); MAX_LIGHTS];
    let mut durations = Vec::new();
    let iterations = 10_000;
    for _ in 0..9 {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(
                scene_light_parameters(black_box(&lights), [96, 52, 96], ShadowMode::Soft).unwrap(),
            );
        }
        durations.push(started.elapsed() / iterations);
    }
    durations.sort();
    eprintln!(
        "32-light validation/resolution/unbiased CDF setup median {:?}",
        durations[durations.len() / 2]
    );
}
