use sanim_core::{Animation, Color, Fraction};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::collections::BTreeMap;
fn animation(source: &str) -> Animation {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", source);
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap()
        .animation
}
fn point(x: f32, y: f32) -> [f32; 2] {
    [960.0 + x * 4.0 / 3.0, 540.0 + y * 4.0 / 3.0]
}
#[test]
fn topmost_visible_native_path_wins_and_keeps_ancestry() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation("let base=Rect(width=60pt,height=60pt)\nlet circle=Circle(radius=20pt)\nlet hidden=Rect(width=60pt,height=60pt,opacity=0)\nlet transparent=Circle(radius=30pt,fill=Color(\"transparent\"))\nlet gradient=Rect(width=60pt,height=60pt,fill=LinearGradient(from=Color(\"transparent\"),to=Color(\"transparent\")))\nlet group=Group(children=[circle,hidden,transparent,gradient])\nadd(base)\nadd(group)\nwait(1s)")).unwrap();
    let options = Options::for_animation(scene.animation());
    let center = renderer
        .pick(&scene, 0.into(), &options, point(0., 0.), 0.)
        .unwrap()
        .unwrap();
    assert_eq!(center.id, "circle");
    assert_eq!(center.ancestors, ["group"]);
    assert_eq!(
        renderer
            .pick(&scene, 0.into(), &options, point(19., 19.), 0.)
            .unwrap()
            .unwrap()
            .id,
        "base"
    );
    assert!(
        renderer
            .pick(&scene, 0.into(), &options, point(40., 0.), 0.)
            .unwrap()
            .is_none()
    );
    assert_eq!(renderer.states_computed, 1);
    assert_eq!(renderer.state_cache_hits, 0);
}
#[test]
fn transform_resize_reveal_and_stroke_match_actual_rendered_coverage() {
    let mut renderer = Renderer::default();
    let mut animation = animation(
        "let line=Line(end=Vec2(50pt,0pt),width=0.75pt,position=Vec2(-25pt,20pt))\nlet shape=Rect(width=50pt,height=30pt,radius=8pt,rotation=30deg)\nlet group=Group(children=[shape,line],rotation=20deg,position=Vec2(10pt,-8pt))\nCreate(group,duration=1s,ease=\"linear\")",
    );
    animation.background = Color::parse("transparent").unwrap();
    let scene = renderer.prepare(animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.width = 480;
    options.height = 270;
    options.encode_png = false;
    for time in [
        Fraction::new(1u64, 4u64),
        Fraction::new(3u64, 4u64),
        1.into(),
    ] {
        let frame = renderer.render(&scene, time, &options).unwrap();
        let computed = renderer.states_computed;
        for y in (100..175).step_by(3) {
            for x in (200..290).step_by(3) {
                let alpha = frame.rgba[(y * 480 + x) * 4 + 3];
                // Ignore antialias edge coverage; native interior/empty samples must agree.
                if alpha > 0 && alpha < 250 {
                    continue;
                }
                let hit = renderer
                    .pick(&scene, time, &options, [x as f32 + 0.5, y as f32 + 0.5], 0.)
                    .unwrap();
                assert_eq!(hit.is_some(), alpha > 0, "pixel {x},{y} at {time}");
            }
        }
        assert_eq!(
            computed, renderer.states_computed,
            "picking reuses the recording's sampled state"
        );
    }
}
#[test]
fn thin_lines_have_device_pixel_tolerance_and_no_invisible_future_hits() {
    let mut renderer = Renderer::default();
    let scene = renderer
        .prepare(animation(
            "wait(1s)\nlet line=Line(end=Vec2(100pt,0pt),width=0.25pt)\nadd(line)\nwait(1s)",
        ))
        .unwrap();
    let options = Options::for_animation(scene.animation());
    assert!(
        renderer
            .pick(&scene, 0.into(), &options, point(30., 0.), 3.)
            .unwrap()
            .is_none()
    );
    assert!(
        renderer
            .pick(&scene, 1.into(), &options, point(30., 1.), 0.)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        renderer
            .pick(&scene, 1.into(), &options, point(30., 1.), 3.)
            .unwrap()
            .unwrap()
            .id,
        "line"
    );
}
#[test]
fn reverse_seek_and_hot_patch_retain_unaffected_prefix_state() {
    let mut renderer = Renderer::default();
    let source = |radius| {
        format!(
            "let first=Circle(radius=10pt)\nadd(first)\nwait(1s)\nlet second=Circle(radius={radius}pt,position=Vec2(50pt,0pt))\nadd(second)\nwait(1s)"
        )
    };
    let old = renderer.prepare(animation(&source(10))).unwrap();
    let options = Options::for_animation(old.animation());
    renderer
        .pick(&old, 0.into(), &options, point(0., 0.), 0.)
        .unwrap();
    renderer
        .pick(&old, 1.into(), &options, point(65., 0.), 0.)
        .unwrap();
    let changed = renderer.prepare(animation(&source(20))).unwrap();
    assert_eq!(
        renderer
            .pick(&changed, 1.into(), &options, point(65., 0.), 0.)
            .unwrap()
            .unwrap()
            .id,
        "second"
    );
    let computed = renderer.states_computed;
    assert_eq!(
        renderer
            .pick(&changed, 0.into(), &options, point(0., 0.), 0.)
            .unwrap()
            .unwrap()
            .id,
        "first"
    );
    assert_eq!(renderer.states_computed, computed);
}
#[test]
#[ignore]
fn picking_benchmark() {
    for count in [1000, 10_000] {
        let prototype = animation("add(Circle(radius=1pt))\nwait(1s)").layers[0]
            .element
            .clone();
        let mut animation = Animation::for_viewport(&Default::default());
        animation.duration = 1.into();
        for index in 0..count {
            let mut element = prototype.clone();
            element.id = format!("dot_{index}");
            element.position.values = [
                (index % 100 * 5 - 250) as f64,
                (index / 100 * 5 - 250) as f64,
            ];
            animation.add(element, 0.into()).unwrap();
        }
        let mut renderer = Renderer::default();
        let scene = renderer.prepare(animation).unwrap();
        let options = Options::for_animation(scene.animation());
        let start = std::time::Instant::now();
        renderer
            .pick(&scene, 0.into(), &options, point(0., 0.), 3.)
            .unwrap();
        let cold = start.elapsed();
        let geometry_visits = renderer.geometry_key_hits;
        let start = std::time::Instant::now();
        const RUNS: u32 = 50;
        for index in 0..RUNS {
            std::hint::black_box(
                renderer
                    .pick(&scene, 0.into(), &options, point(index as f32, 0.), 3.)
                    .unwrap(),
            );
        }
        println!(
            "picking {count} shapes: cold={cold:?}, warm/click={:?}, states={}, geometry={}",
            start.elapsed() / RUNS,
            renderer.states_computed,
            renderer.geometries_prepared
        );
        assert_eq!(renderer.states_computed, 1);
        assert_eq!(
            renderer.geometry_key_hits, geometry_visits,
            "cached clicks must not traverse and prepare native draw geometry again"
        );
    }
}

#[test]
fn nested_parent_relative_layout_tracks_external_canvas_resizing() {
    let mut renderer = Renderer::default();
    let scene=renderer.prepare(animation("let marker=Circle(radius=10pt,position=Vec2(25%,0%))\nadd(Rect(width=50%,height=50%,fill=Color(\"transparent\"),children=[marker]))\nwait(1s)")).unwrap();
    let mut options = Options::for_animation(scene.animation());
    for width in [960u32, 1920] {
        options.width = width;
        options.logical_width = Fraction::new(u64::from(width) * 72, 96u64);
        let hit = renderer
            .pick(&scene, 0.into(), &options, [width as f32 * 0.625, 540.], 0.)
            .unwrap()
            .unwrap();
        assert_eq!(hit.id, "marker");
        assert_eq!(hit.ancestors.len(), 1);
        assert!(
            renderer
                .pick(&scene, 0.into(), &options, [width as f32 * 0.75, 540.], 0.)
                .unwrap()
                .is_none()
        );
    }
}
