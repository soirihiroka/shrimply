use sanim_core::{Animation, Fraction};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};
use std::{collections::BTreeMap, sync::Arc};

fn animation(source: &str) -> Animation {
    let mut sources = MemorySources::default();
    sources.insert("main.sanim", format!("{}\nAnimation()", source));
    Engine::default()
        .prepare("main.sanim", &sources, &BTreeMap::new())
        .unwrap()
        .animation
}

#[test]
fn future_and_transparent_geometry_is_prepared_only_when_drawn() {
    let animation = animation(
        "Animation(children=[Box(width=2*(10pt), height=2*(10pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])\nAnimation(children=[Box(width=17pt, height=9pt, opacity=0, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])\nwait(1s)\nAnimation(children=[Box(width=20pt, height=12pt, fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)])\nwait(1s)",
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation).unwrap();
    let options = Options::for_animation(scene.animation());
    assert_eq!(renderer.geometries_prepared, 0);
    let early = renderer.record(&scene, 0.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    renderer.record(&scene, 1.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 2);
    let reverse = renderer.record(&scene, 0.into(), &options).unwrap();
    assert!(Arc::ptr_eq(&early, &reverse));
    renderer.validate_geometry(&scene).unwrap();
    assert_eq!(renderer.geometries_prepared, 3);
}

#[test]
fn stable_geometry_avoids_hashing_again_on_a_new_animated_frame() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation("let marker=Box(width=2*(10pt), height=2*(10pt), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nmarker.move_to([30pt, 0pt], duration=1s, ease=\"linear\")")).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    assert_eq!(renderer.geometry_key_hits, 0);
    renderer
        .record(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    assert_eq!(renderer.geometry_key_hits, 1);
}

#[test]
fn many_elements_sharing_one_path_keep_their_identity_hints_between_frames() {
    const ELEMENTS: usize = 1500;
    let circles = (0..ELEMENTS)
        .map(|index| format!("Box(id=\"dot_{index}\", width=2pt,height=2pt,radius=\"full\",fill=Color(\"light1\"))"))
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        "let dots=Box(children=[{circles}])\ndots.move_to([30pt, 0pt], duration=1s, ease=\"linear\")"
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(&source)).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    let identity_hits = renderer.geometry_key_hits;
    renderer
        .record(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    assert_eq!(renderer.geometries_prepared, 1);
    assert_eq!(renderer.geometry_key_hits - identity_hits, ELEMENTS as u64);
}

#[test]
fn exceeding_the_identity_hint_budget_keeps_admitted_geometry_cached() {
    const ELEMENTS: usize = 9000;
    let source = format!(
        "fn marker(index:Number):Element{{return Box(id=\"marker_{{index}}\",width=2pt,height=2pt,radius=\"full\",fill=Color(\"light1\"))}}\nlet markers=Box(children=range({ELEMENTS}).map(marker))\nmarkers.move_to([30pt, 0pt], duration=1s, ease=\"linear\")"
    );
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation(&source)).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    let before = renderer.geometry_key_hits;
    renderer
        .record(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    // Overflow must preserve existing admissions, rather than flushing all hints
    // repeatedly and rehashing every element on every subsequent frame.
    assert!(renderer.geometry_key_hits - before > ELEMENTS as u64 / 2);
    assert_eq!(renderer.geometries_prepared, 1);
}

#[test]
fn dimensions_use_current_geometry_and_reuse_previous_content_on_reverse_seek() {
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(animation("mut radius=10pt\nlet marker=Box(width=2*(radius), height=2*(radius), radius=\"full\", fill=Color(\"light1\"), x_align=Align.Center, y_align=Align.Center)\nAnimation(children=[marker])\nanimate(radius,to=20pt,duration=1s,ease=\"linear\")")).unwrap();
    let options = Options::for_animation(scene.animation());
    renderer.record(&scene, 0.into(), &options).unwrap();
    renderer
        .record(&scene, Fraction::new(1u64, 2u64), &options)
        .unwrap();
    assert_eq!(renderer.geometries_prepared, 2);
    renderer.record(&scene, 1.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 3);
    renderer.record(&scene, 0.into(), &options).unwrap();
    assert_eq!(renderer.geometries_prepared, 3);
}
