use imbl::OrdMap;
use sanim_core::voxel::camera::{Op, Spec, execute};

#[test]
fn default_camera_percentages_resolve_to_an_external_world_space_view() {
    let value = execute(Op::Camera, &OrdMap::new()).unwrap();
    let camera = Spec::new(&value)
        .unwrap()
        .resolve([100; 3], [1920, 1080])
        .unwrap();
    assert_eq!(camera.position, [120., 70., 120.]);
    assert_eq!(camera.target, [50., 25., 50.]);
}
