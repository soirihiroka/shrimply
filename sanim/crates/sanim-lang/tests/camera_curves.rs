use sanim_core::{Fraction, Geometry, ListLength, Type, Unit};
use sanim_lang::{Engine, MemorySources};

fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}

#[test]
fn curves_are_typed_list_paths_sampled_by_distance_without_camera_flags() {
    let prepared = prepare(
        r#"
let route:Curve<List<Number,2>>=Curve(points=[[0,0],[2,0],[10,0]],interpolation="linear")
let middle=route.at(0.5)
assert(middle==[5,0])
let tangent=route.direction(0.5)
assert(tangent==[1,0])
Animation()
"#,
    )
    .unwrap();
    assert_eq!(
        prepared.bindings["middle"].ty(),
        Type::List(Box::new(Type::Numeric(Unit::Number)), ListLength::Exact(2))
    );
    assert!(prepare("let route=Curve(points=[[0,0],[1,1,1]])\nAnimation()").is_err());
    assert!(
        prepare("let route=Curve(points=[[0,0],[1,1]],constant_speed=true)\nAnimation()").is_err()
    );
}

fn camera_value(expression: &str) -> sanim_core::voxel::camera::Spec {
    let prepared = prepare(&format!("let shot={expression}\nAnimation()")).unwrap();
    sanim_core::voxel::camera::Spec::new(&prepared.bindings["shot"]).unwrap()
}

#[test]
fn pan_dolly_zoom_and_orbit_have_distinct_geometric_meanings() {
    let base = "Camera(position=[0,0,10],target=[0,0,0],fov=60deg)";
    let camera = camera_value(&format!("{base}.pan([2,3])"))
        .resolve([100; 3], [800, 600])
        .unwrap();
    assert_eq!(camera.position, [2.0, 3.0, 10.0]);
    assert_eq!(camera.target, [2.0, 3.0, 0.0]);
    let camera = camera_value(&format!("{base}.dolly(2)"))
        .resolve([100; 3], [800, 600])
        .unwrap();
    assert_eq!(camera.position, [0.0, 0.0, 8.0]);
    let old = camera_value(base).resolve([100; 3], [800, 600]).unwrap();
    let new = camera_value(&format!("{base}.dolly_zoom(2)"))
        .resolve([100; 3], [800, 600])
        .unwrap();
    let a = old.basis([800, 600]).unwrap();
    let b = new.basis([800, 600]).unwrap();
    assert!((f64::from(a.half_height) * 10.0 - f64::from(b.half_height) * 8.0).abs() < 1e-5);
    let orbit =
        camera_value("OrbitCamera(target=[50%,10%,50%],radius=20,azimuth=90deg,elevation=0deg)")
            .resolve([100, 50, 80], [800, 600])
            .unwrap();
    assert_eq!(orbit.target, [50.0, 5.0, 40.0]);
    assert_eq!(orbit.position, [70.0, 5.0, 40.0]);
}

#[test]
fn relative_curve_camera_signals_seek_in_any_order_without_rebuilding_geometry() {
    let prepared = prepare(
        r#"
let route=Curve(points=[[120%,60%,120%],[100%,40%,90%],[20%,30%,80%]])
mut progress=0
sig shot=Camera(path=route,progress=progress,target=[50%,25%,50%],fov=50deg).zoom(2)
let scene=VoxelScene(volume=VoxelGrid(size=[32,24,40]),camera=shot)
scene.fade_in(duration=100ms)
animate(progress,to=1,duration=1s,ease="linear")
wait(1s)
"#,
    )
    .unwrap();
    let scene = |time: Fraction| {
        let elements = prepared.animation.sample(time).unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel scene")
        };
        scene.clone()
    };
    let late = scene(Fraction::from(1));
    let early = scene(Fraction::new(1u64, 5u64));
    assert_ne!(
        late.resolve_camera([640, 360]).unwrap().position,
        early.resolve_camera([640, 360]).unwrap().position
    );
    assert_eq!(
        late.resolve_camera([640, 360]).unwrap(),
        scene(Fraction::from(1)).resolve_camera([640, 360]).unwrap()
    );
    assert!(late.source.same_snapshot(&early.source));
    let error=prepare("let scene=VoxelScene(volume=VoxelGrid(size=[8,8,8]),camera=Camera(),yaw=20deg)\nscene.fade_in()").unwrap_err();
    assert!(error.message.contains("camera"));
}
