use sanim_core::{Fraction, Geometry, Value};
use sanim_lang::{Engine, MemorySources};
use std::sync::Arc;

#[test]
fn moving_instance_layout_retains_original_assets_and_never_materializes_a_world_grid() {
    let mut files = MemorySources::default();
    files.insert("main.sanim",r#"
let asset=VoxelGrid(size=[8,8,8]).fill([0,0,0],[8,8,8],Color("blue3"))
let copies=Instances(asset=asset,positions=[[0,0,0],[8,0,0]],size=[16,8,8])
mut shift=0
sig layout=<VoxelBox size={[32,8,8]}><VoxelBox size={[16,8,8]} position_mode="absolute" offset={[shift,0,0]}>{copies}</VoxelBox></VoxelBox>
let scene=VoxelScene(layout=layout)
scene.fade_in(duration=1ms)
animate(shift,to=8,duration=1s,ease="linear")
"#);
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let Value::VoxelGrid(asset) = &prepared.bindings["asset"] else {
        panic!("asset");
    };
    let frames = [
        Fraction::new(1u64, 1000u64),
        Fraction::new(1001u64, 1000u64),
    ]
    .map(|time| prepared.animation.sample(time).unwrap());
    let scenes = frames
        .each_ref()
        .map(|elements| match elements[0].geometry.as_ref() {
            Geometry::Voxel(scene) => scene,
            _ => panic!("scene"),
        });
    let instances = scenes.map(|scene| match &scene.source {
        sanim_voxel::SceneSource::Instances(instances) => instances,
        _ => panic!("layout must stay instanced"),
    });
    assert_eq!(instances[0].instances()[0].origin, [0, 0, 0]);
    assert_eq!(instances[1].instances()[0].origin, [8, 0, 0]);
    assert_eq!(instances[0].instances().len(), 2);
    for scene in instances {
        for instance in scene.instances() {
            assert!(Arc::ptr_eq(asset, &instance.volume));
        }
    }
    assert!(scenes.iter().all(|scene| scene.occupied_voxels().is_none()));
    assert_eq!(scenes[1].source.get([8, 0, 0]).unwrap(), 1);
    assert_eq!(scenes[1].source.get([0, 0, 0]).unwrap(), 0);
}
