use sanim_core::{Fraction, Geometry, ListLength, Type, Unit};
use sanim_lang::{Engine, MemorySources};
fn prepare(source: &str) -> Result<sanim_lang::Prepared, sanim_core::Diagnostic> {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default().prepare("main.sanim", &sources, &Default::default())
}
#[test]
fn keyframes_are_typed_and_share_exact_clocks_with_manual_bezier_handles() {
    let p=prepare(r#"
let linear:Curve<Number>=Curve(keys=[Keyframe(time=0s,value=0,interpolation="linear"),Keyframe(time=2s,value=10)])
assert(linear.duration()==2s)
assert(linear.sample(500ms)==2.5)
assert(linear.at(0.5)==5)
let hold=Curve(keys=[Keyframe(time=0s,value=2,interpolation="hold"),Keyframe(time=1s,value=9)])
assert(hold.sample(999ms)==2)
assert(hold.sample(1s)==9)
let manual=Curve(keys=[Keyframe(time=0s,value=0,handle_out={time=0s,value=0},tangents="manual"),Keyframe(time=1s,value=1,handle_in={time=0s,value=-1},tangents="manual")])
assert(abs(manual.at(0.5)-0.125)<0.00001)
let path=Curve(keys=[Keyframe(time=0s,value=[0pt,10pt]),Keyframe(time=1s,value=[20pt,30pt])])
let middle=path.sample(500ms)
assert(middle==[10pt,20pt])
Animation()
"#).unwrap();
    assert_eq!(
        p.bindings["middle"].ty(),
        Type::List(Box::new(Type::Numeric(Unit::Length)), ListLength::Exact(2))
    );
}
#[test]
fn keyframed_values_and_camera_seek_out_of_order_without_rebuilding_voxels() {
    let p=prepare(r#"
let position_curve=Curve(keys=[Keyframe(time=0s,value=[120%,60%,120%]),Keyframe(time=2s,value=[90%,40%,90%])])
let zoom_curve=Curve(keys=[Keyframe(time=0s,value=1),Keyframe(time=2s,value=2)])
mut progress=0
sig shot=Camera(position=position_curve.at(progress),target=[50%,25%,50%],fov=50deg).zoom(zoom_curve.at(progress))
let scene=VoxelScene(volume=VoxelGrid(size=[32,24,40]),camera=shot)
scene.fade_in(duration=0s)
let movement=Curve(keys=[Keyframe(time=0s,value=0),Keyframe(time=2s,value=1)])
animate(progress,curve=movement)
wait(1s)
"#).unwrap();
    assert_eq!(p.animation.duration, Fraction::from(3));
    let scene = |time| {
        let elements = p.animation.sample(time).unwrap();
        let Geometry::Voxel(scene) = elements[0].geometry.as_ref() else {
            panic!("voxel")
        };
        scene.clone()
    };
    let later = scene(2.into());
    let early = scene(Fraction::new(1u64, 2u64));
    assert_ne!(
        later.resolve_camera([640, 360]).unwrap(),
        early.resolve_camera([640, 360]).unwrap()
    );
    assert_eq!(
        later.resolve_camera([640, 360]).unwrap(),
        scene(2.into()).resolve_camera([640, 360]).unwrap()
    );
    assert!(later.source.same_snapshot(&early.source));
    p.animation.reactive.validate().unwrap();
}
#[test]
fn invalid_key_shapes_clocks_handles_and_animation_arguments_fail_cleanly() {
    for (source, message) in [
        (
            "let c=Curve(keys=[Keyframe(time=1s,value=0),Keyframe(time=2s,value=1)])",
            "0s",
        ),
        (
            "let c=Curve(keys=[Keyframe(time=0s,value=0),Keyframe(time=0s,value=1)])",
            "strictly after",
        ),
        (
            "let c=Curve(keys=[Keyframe(time=0s,value=[1,2]),Keyframe(time=1s,value=[1,2,3])])",
            "List",
        ),
        (
            "let c=Keyframe(time=0s,value=1pt,handle_out={time=1s,value=1})",
            "units",
        ),
        (
            "let c=Curve(keys=[Keyframe(time=0s,value=0,handle_out={time=2s,value=0}),Keyframe(time=1s,value=1)])",
            "time handles",
        ),
        ("mut x=0\nanimate(x,to=1)", "duration"),
        (
            "mut x=0\nlet c=Curve(keys=[Keyframe(time=0s,value=0),Keyframe(time=1s,value=1)])\nanimate(x,to=1,curve=c)",
            "exactly one",
        ),
        (
            "mut x=0pt\nlet c=Curve(keys=[Keyframe(time=0s,value=0),Keyframe(time=1s,value=1)])\nanimate(x,curve=c)",
            "type mismatch",
        ),
    ] {
        let error = prepare(&format!("{source}\nAnimation()")).unwrap_err();
        assert!(error.message.contains(message), "{source}: {error}");
    }
}

#[test]
fn inline_constant_curves_share_the_compilation_while_progress_changes() {
    let source = r#"mut progress=0
sig width=Curve(keys=[Keyframe(time=0s,value=10pt),Keyframe(time=1s,value=30pt)]).at(progress)
let shape=Box(width=width,height=10pt)
shape.fade_in(duration=0s)
animate(progress,to=1,duration=1s,ease="linear")
Animation()"#;
    let p = prepare(source).unwrap();
    let graph = &p.animation.reactive;
    let mut early = graph.sampler(0.into()).unwrap();
    let mut later = graph.sampler(1.into()).unwrap();
    let mut compiled = 0;
    for id in 0..graph.node_count() {
        if let sanim_core::Value::Curve(a) = early.value(id).unwrap() {
            let sanim_core::Value::Curve(b) = later.value(id).unwrap() else {
                panic!("Curve")
            };
            assert!(std::sync::Arc::ptr_eq(&a, &b));
            compiled += 1;
        }
    }
    assert_eq!(compiled, 1);
    for (time, width) in [
        (Fraction::from(0), 10),
        (Fraction::new(1u64, 2u64), 20),
        (Fraction::from(1), 30),
    ] {
        assert_eq!(
            p.animation.sample(time).unwrap()[0]
                .property(sanim_core::Property::Width)
                .unwrap()
                .number()
                .unwrap()
                .value,
            Fraction::from(width)
        );
    }
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    let mut engine = Engine::default();
    engine.limits.max_native_work = p.statistics.native_work - 1;
    let error = engine
        .prepare("main.sanim", &sources, &Default::default())
        .unwrap_err();
    assert!(
        error.message.contains("native") && error.message.contains("budget"),
        "{error}"
    );
}

#[test]
#[ignore = "run make curve-inline-bench"]
fn inline_curve_compile_once_benchmark() {
    let keys = (0..128)
        .map(|i| format!("Keyframe(time={i}s,value={}pt)", 10 + i % 7))
        .collect::<Vec<_>>()
        .join(",");
    for inline in [false, true] {
        let curve = format!("Curve(keys=[{keys}])");
        let source = if inline {
            format!("mut p=0\nsig width={curve}.at(p)\n")
        } else {
            format!("let curve={curve}\nmut p=0\nsig width=curve.at(p)\n")
        };
        let prepared=prepare(&format!("{source}let shape=Box(width=width,height=10pt)\nshape.fade_in(duration=0s)\nanimate(p,to=1,duration=1s,ease=\"linear\")\nAnimation()")).unwrap();
        let graph = &prepared.animation.reactive;
        let width = graph.binding("shape", sanim_core::Property::Width).unwrap();
        let start = std::time::Instant::now();
        let mut checksum = 0.0;
        for tick in 0..2000u64 {
            checksum += sanim_core::math::to_f64(
                graph
                    .sampler(Fraction::new(tick, 2000u64))
                    .unwrap()
                    .value(width)
                    .unwrap()
                    .number()
                    .unwrap()
                    .value,
            )
            .unwrap();
        }
        let elapsed = start.elapsed();
        if inline {
            let mut sampler = graph.sampler(0.into()).unwrap();
            let keys=(0..graph.node_count()).find_map(|id| match graph.node_type(id) {
                Some(Type::List(inner,_)) if matches!(inner.as_ref(),Type::Record(fields) if fields.contains_key("time") && fields.contains_key("value")) => Some(sampler.value(id).unwrap()),
                _=>None,
            }).expect("constant Keyframe List");
            let args = [("keys".to_owned(), keys)].into_iter().collect();
            let start = std::time::Instant::now();
            let mut rebuilt_checksum = 0.0;
            for tick in 0..2000u64 {
                let sanim_core::Value::Curve(curve) =
                    sanim_core::voxel::camera::execute(sanim_core::voxel::camera::Op::Curve, &args)
                        .unwrap()
                else {
                    panic!("Curve")
                };
                rebuilt_checksum += sanim_core::math::to_f64(
                    curve
                        .at(Fraction::new(tick, 2000u64))
                        .unwrap()
                        .number()
                        .unwrap()
                        .value,
                )
                .unwrap();
            }
            assert_eq!(checksum, rebuilt_checksum);
            println!(
                "recompile_only=true keys=128 frames=2000 elapsed={:?} checksum={rebuilt_checksum}",
                start.elapsed()
            );
        }
        println!(
            "inline={inline} keys=128 frames=2000 nodes={} elapsed={:?} checksum={checksum}",
            graph.node_count(),
            elapsed
        );
    }
}

#[test]
fn folding_invalid_constants_preserves_short_circuit_laziness() {
    let p = prepare(
        r#"mut enabled=false
sig numbers=enabled and Color("no-such-color")==Color("blue3")
let code=CodeView(code="x",lang="plain",line_numbers=numbers)
code.fade_in(duration=0s)
wait(1s)
Animation()"#,
    )
    .unwrap();
    p.animation.sample(0.into()).unwrap();
    p.animation.sample(1.into()).unwrap();
}
