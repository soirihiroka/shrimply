use sanim_voxel::{Camera, CameraRig, Projection, View};
use std::sync::Arc;

#[test]
fn inspection_views_have_stable_center_rays_and_non_singular_top_and_bottom() {
    for view in [
        View::Front,
        View::Back,
        View::Left,
        View::Right,
        View::Top,
        View::Bottom,
        View::Isometric,
    ] {
        assert_eq!(view.to_string().parse::<View>().unwrap(), view);
        for viewport in [[101, 101], [201, 101], [101, 201]] {
            let camera = Camera::fit(view, [8, 12, 16], viewport).unwrap();
            let ray = camera
                .ray_for_pixel(viewport, viewport.map(|size| size / 2))
                .unwrap();
            let target = [4.0, 6.0, 8.0];
            let delta = std::array::from_fn::<_, 3, _>(|axis| target[axis] - ray.origin[axis]);
            let distance = delta.iter().map(|value| value * value).sum::<f64>().sqrt();
            for (axis, value) in delta.iter().enumerate() {
                assert!(
                    (*value / distance - ray.direction[axis]).abs() < 1e-5,
                    "{view}"
                );
            }
        }
    }
}

#[test]
fn fitting_and_projection_reject_invalid_inputs() {
    assert!(Camera::fit(View::Front, [0, 1, 1], [10, 10]).is_err());
    assert!(
        CameraRig {
            zoom: 0.0,
            ..Default::default()
        }
        .resolve([8; 3], [100; 2])
        .is_err()
    );
    assert!(
        CameraRig {
            vertical_fov_degrees: Some(180.0),
            ..Default::default()
        }
        .resolve([8; 3], [100; 2])
        .is_err()
    );
    let mut camera = Camera::fit(View::Front, [8; 3], [100; 2]).unwrap();
    camera.up = [0.0, 0.0, 1.0];
    assert!(camera.basis([100; 2]).is_err());
    camera.up = [0.0, 1.0, 0.0];
    camera.projection = Projection::Orthographic { height: f32::NAN };
    assert!(camera.ray_for_pixel([100; 2], [50; 2]).is_err());
    camera.position = [f32::MAX, 0.0, 0.0];
    camera.target = [-f32::MAX, 0.0, 0.0];
    camera.projection = Projection::Orthographic { height: 1.0 };
    let basis = camera.basis([100; 2]).unwrap();
    assert!(
        basis
            .forward
            .iter()
            .chain(&basis.right)
            .chain(&basis.up)
            .all(|value| value.is_finite())
    );
}

#[test]
fn material_plan_rejects_mixed_tile_sizes_without_a_gpu() {
    use sanim_voxel::{Material, Texture, Volume};
    let small = Arc::new(Texture::new(1, 1, Arc::from([255; 4])).unwrap());
    let large = Arc::new(Texture::new(2, 1, Arc::from([255; 8])).unwrap());
    let material = Arc::new(
        Material::textured(
            0xffffffff,
            [Some(small), Some(large), None, None, None, None],
        )
        .unwrap(),
    );
    let (volume, _) = Volume::new([8; 3])
        .unwrap()
        .with_material(material)
        .unwrap();
    assert!(
        sanim_voxel_render::validate_materials(volume.materials())
            .unwrap_err()
            .contains("matching dimensions")
    );
    let invalid = sanim_voxel_render::RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        samples: 0,
        ..Default::default()
    };
    assert!(invalid.validate().is_err());
}

#[test]
fn path_work_budget_checks_exact_boundary_and_overflow_without_a_gpu() {
    use sanim_voxel_render::{RenderMode, RenderSettings};
    let exact = RenderSettings {
        reconstruction: sanim_voxel_render::Reconstruction {
            mode: sanim_voxel_render::ReconstructionMode::Off,
            antialias_samples: 1,
            ..Default::default()
        },
        width: 4096,
        height: 4096,
        samples: 4,
        bounces: 8,
        mode: RenderMode::PathTraced,
        ..Default::default()
    };
    assert!(exact.validate().is_ok());
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            width: 4097,
            ..exact.clone()
        }
        .validate()
        .unwrap_err()
        .contains("path-segment budget")
    );
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            width: u32::MAX,
            height: u32::MAX,
            samples: 256,
            ..exact.clone()
        }
        .validate()
        .is_err()
    );
    assert!(
        RenderSettings {
            reconstruction: sanim_voxel_render::Reconstruction {
                mode: sanim_voxel_render::ReconstructionMode::Off,
                antialias_samples: 1,
                ..Default::default()
            },
            width: 4097,
            mode: RenderMode::Flat,
            ..exact
        }
        .validate()
        .is_ok()
    );
}
