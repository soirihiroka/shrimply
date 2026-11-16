use sanim_core::{Fraction, Geometry, Value};
use sanim_lang::{Engine, MemorySources};
fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut files = MemorySources::default();
    files.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &files, &Default::default())
}

#[test]
fn typed_light_records_share_signal_configuration_and_keep_voxel_payload_immutable() {
    let prepared = prepare(
        r#"
mut brightness=0
let sun:SunLight=SunLight(direction=[-0.4,0.8,0.5],diameter=8deg)
sig lamp=PointLight(position=[50%,12,25%],color=Color("orange3"),intensity=brightness,radius=0.5)
sig sky=Environment(top=Color("blue3"),intensity=brightness,visible=true)
let scene=VoxelScene(volume=VoxelGrid(size=[20,10,40]),lights=[sun,lamp],environment=sky)
scene.fade_in(duration=100ms)
animate(brightness,to=200,duration=1s,ease="linear")
"#,
    )
    .unwrap();
    let frames = [Fraction::new(1u64, 10u64), Fraction::new(11u64, 10u64)]
        .map(|time| prepared.animation.sample(time).unwrap());
    let scenes = frames
        .each_ref()
        .map(|elements| match elements[0].geometry.as_ref() {
            Geometry::Voxel(scene) => scene,
            _ => panic!("voxel"),
        });
    let lamps = scenes.map(|scene| match scene.lights.as_ref().unwrap()[1] {
        sanim_voxel::Light::Point(light) => light,
        _ => panic!("point"),
    });
    assert_eq!(lamps[0].intensity, 0.);
    assert_eq!(lamps[1].intensity, 200.);
    assert_eq!(scenes[1].environment.unwrap().intensity, 200.);
    assert!(scenes[0].source.same_snapshot(&scenes[1].source));
    assert_eq!(
        lamps[1].position[0],
        sanim_voxel::layout::Coordinate::Percent(50.into())
    );
}
#[test]
fn omitted_lights_keep_default_sun_and_explicit_empty_lights_disable_it() {
    let prepared=prepare("let legacy=VoxelScene(volume=VoxelGrid(size=[4,4,4]))\nlet dark=VoxelScene(volume=VoxelGrid(size=[4,4,4]),lights=[],environment=Environment(intensity=0,visible=false))\nAnimation()").unwrap();
    let scene = |name| {
        let Value::Element(element) = &prepared.bindings[name] else {
            panic!("element")
        };
        let Geometry::Voxel(scene) = element.geometry.as_ref() else {
            panic!("scene")
        };
        scene
    };
    assert!(scene("legacy").lights.is_none());
    assert!(scene("dark").lights.as_ref().unwrap().is_empty());
    assert!(!scene("dark").environment.unwrap().visible);
}
#[test]
fn light_types_ranges_and_static_count_errors_are_readable() {
    for (source, message) in [
        (
            "let light=PointLight(position=[1s,2,3])\nAnimation()",
            "Percent",
        ),
        (
            "let light=SunLight(direction=[0,0,0])\nAnimation()",
            "nonzero",
        ),
        (
            "let light=PointLight(intensity=-1)\nAnimation()",
            "intensity",
        ),
        ("let light=Environment(visible=1)\nAnimation()", "Bool"),
    ] {
        let error = prepare(source).unwrap_err();
        assert!(error.message.contains(message), "{error}");
    }
    let source = format!(
        "let light=SunLight();let scene=VoxelScene(volume=VoxelGrid(size=[1,1,1]),lights=[{}]);Animation()",
        vec!["light"; 33].join(",")
    );
    assert!(prepare(&source).unwrap_err().message.contains("maximum 32"));
}
