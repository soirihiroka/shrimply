use sanim_core::{Fraction, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};

fn scene() -> (Renderer, sanim_render::Scene, Options) {
    let mut files = MemorySources::default();
    files.insert(
        "main.sanim",
        r#"mut count=0
mut size=14pt
mut typeface=Font("sans-serif")
let title=<Label font={typeface} font_size={size}>Count = {count:.0f}</Label>
add(title)
background {animate(count,to=10,duration=1s,ease="linear")}
background {animate(size,to=30pt,duration=1s,ease="linear")}
wait(500ms)
typeface=Font("serif")
wait(500ms)
"#,
    );
    let animation = Engine::new(Viewport {
        width: 360,
        height: 220,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap()
    .animation;
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    (renderer, scene, options)
}

#[test]
fn reactive_label_frames_shape_only_when_needed_and_survive_reverse_scrubbing() {
    let (mut renderer, scene, options) = scene();
    let first = renderer.render(&scene, 0.into(), &options).unwrap();
    let middle = renderer
        .render(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    let last = renderer.render(&scene, 1.into(), &options).unwrap();
    assert_ne!(first.rgba, middle.rgba);
    assert_ne!(middle.rgba, last.rgba);
    let states = renderer.states_computed;
    let geometry = renderer.geometries_prepared;
    let replay = renderer.render(&scene, 0.into(), &options).unwrap();
    assert_eq!(replay.rgba, first.rgba);
    assert_eq!(renderer.states_computed, states);
    assert_eq!(renderer.geometries_prepared, geometry);
}

#[test]
fn invalid_signal_content_fails_only_its_frame_and_preserves_previous_cached_text() {
    let mut files = MemorySources::default();
    files.insert(
        "font.ttf",
        include_bytes!("../../../examples/assets/DejaVuSansCondensed-subset.ttf"),
    );
    files.insert("main.sanim", "mut words=\"safe\"\nlet title=Label(words,font=read<Font>(\"font.ttf\"))\nadd(title)\nwait(1s)\nwords=\"Ω\"\nwait(1s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    let good = renderer.render(&scene, 0.into(), &options).unwrap();
    let error = match renderer.render(&scene, 1.into(), &options) {
        Err(error) => error,
        Ok(_) => panic!("expected missing glyph diagnostic"),
    };
    assert!(error.contains("missing glyph"), "{error}");
    assert_eq!(
        renderer.render(&scene, 0.into(), &options).unwrap().rgba,
        good.rgba
    );
}

#[test]
fn changing_text_bounds_reflows_siblings_before_recording_and_picking() {
    let mut files = MemorySources::default();
    files.insert("main.sanim", "mut words=\"short\"\nlet title=Label(words)\nlet marker=Circle(radius=5pt)\nlet row=Box(flex_direction=\"row\",children=[title,marker],gap=10pt)\nadd(row)\nwait(1s)\nwords=\"much longer text\"\nwait(1s)\n");
    let prepared = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let options = Options::for_animation(scene.animation());
    let before = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    let after = renderer.frame_state(&scene, 1.into(), &options).unwrap();
    let before_marker = before.elements()[0]
        .geometry
        .children()
        .unwrap()
        .iter()
        .find(|child| child.id == "marker")
        .unwrap();
    let after_marker = after.elements()[0]
        .geometry
        .children()
        .unwrap()
        .iter()
        .find(|child| child.id == "marker")
        .unwrap();
    assert!(after_marker.position.values[0] > before_marker.position.values[0]);
    assert!(matches!(
        after.elements()[0].geometry.children().unwrap()[0]
            .geometry
            .as_ref(),
        sanim_core::Geometry::Text { pending: false, .. }
    ));
}

#[test]
#[ignore = "manual performance measurement"]
fn reactive_label_benchmark() {
    use std::{hint::black_box, time::Instant};
    let (mut renderer, scene, options) = scene();
    let started = Instant::now();
    for index in 0..30u64 {
        black_box(
            renderer
                .record(&scene, Fraction::new(index, 30u64), &options)
                .unwrap(),
        );
    }
    let cold = started.elapsed();
    let shapes = renderer.geometries_prepared;
    let started = Instant::now();
    for index in (0..30u64).rev() {
        black_box(
            renderer
                .record(&scene, Fraction::new(index, 30u64), &options)
                .unwrap(),
        );
    }
    assert_eq!(renderer.geometries_prepared, shapes);
    let replay = started.elapsed();
    let mut fresh = Renderer::default();
    let started = Instant::now();
    for index in 0..30u64 {
        black_box(
            fresh
                .record(&scene, Fraction::new(index, 30u64), &options)
                .unwrap(),
        );
    }
    eprintln!(
        "reactive label: 30 changing text/size/font samples={cold:?}; cached reverse={replay:?}; new renderer with shared warm outlines={:?}; native paths={shapes}",
        started.elapsed()
    );
}
