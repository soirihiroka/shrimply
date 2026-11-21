use sanim_core::{Fraction, Geometry};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::sync::Arc;

fn scene(source: &str, renderer: &mut Renderer) -> sanim_render::Scene {
    let mut files = MemorySources::default();
    files.insert("main.sanim", format!("{}\nAnimation()", source));
    let animation = Engine::default()
        .prepare("main.sanim", &files, &Default::default())
        .unwrap()
        .animation;
    renderer.prepare(animation).unwrap()
}

#[test]
fn moving_text_reuses_outline_identity_across_new_geometry_presentations() {
    let mut renderer = Renderer::default();
    let scene = scene(
        "let label=Text(\"Stable outline\",id=\"label\")\nlabel.move_to([100pt, 0pt],duration=1s,ease=\"linear\")",
        &mut renderer,
    );
    let options = Options::for_animation(scene.animation());
    let first = renderer
        .frame_state(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    renderer
        .record(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    let hits = renderer.geometry_key_hits;
    let paths = renderer.geometries_prepared;
    let second = renderer
        .frame_state(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    renderer
        .record(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    let a = first.elements()[0].find("label").unwrap();
    let b = second.elements()[0].find("label").unwrap();
    let (Geometry::Text { outline: a, .. }, Geometry::Text { outline: b, .. }) =
        (a.geometry.as_ref(), b.geometry.as_ref())
    else {
        panic!("Text")
    };
    assert!(
        Arc::ptr_eq(a, b),
        "presentation changes must share the shaped outline"
    );
    assert_eq!(renderer.geometries_prepared, paths);
    assert_eq!(
        renderer.geometry_key_hits,
        hits + 1,
        "a new frame must skip outline serialization and hashing"
    );
}

#[test]
fn changing_sibling_preserves_text_identity_inside_an_active_container() {
    let mut renderer = Renderer::default();
    let scene = scene(
        "mut width=10pt\nlet label=Text(\"Stable sibling\",id=\"label\")\nlet sibling=Box(width=width,height=10pt,fill=Color(\"blue3\"))\nlet panel=Box(width=300pt,height=100pt,children=[label,sibling])\nAnimation(children=[panel])\nanimate(width,to=30pt,duration=1s,ease=\"linear\")",
        &mut renderer,
    );
    let options = Options::for_animation(scene.animation());
    renderer
        .record(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    let hits = renderer.geometry_key_hits;
    renderer
        .record(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    assert!(
        renderer.geometry_key_hits > hits,
        "the unchanged Text must hit its identity hint while its sibling changes"
    );
}

#[test]
fn changed_outline_never_reuses_the_previous_text_path() {
    let mut renderer = Renderer::default();
    let scene = scene(
        "mut words=\"First\"\nlet label=Text(words,id=\"label\")\nAnimation(children=[label])\nwait(500ms)\nanimate(words,to=\"Different words\",duration=0s)\nwait(500ms)",
        &mut renderer,
    );
    let options = Options::for_animation(scene.animation());
    let first = renderer
        .frame_state(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    renderer
        .record(&scene, Fraction::new(1u64, 4u64), &options)
        .unwrap();
    let paths = renderer.geometries_prepared;
    let second = renderer
        .frame_state(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    let actual = renderer
        .render(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    let a = first.elements()[0].find("label").unwrap();
    let b = second.elements()[0].find("label").unwrap();
    let (Geometry::Text { outline: a, .. }, Geometry::Text { outline: b, .. }) =
        (a.geometry.as_ref(), b.geometry.as_ref())
    else {
        panic!("Text")
    };
    assert!(!Arc::ptr_eq(a, b));
    assert_eq!(renderer.geometries_prepared, paths + 1);
    let expected = Renderer::default()
        .render(&scene, Fraction::new(3u64, 4u64), &options)
        .unwrap();
    assert_eq!(actual.rgba, expected.rgba);
}
