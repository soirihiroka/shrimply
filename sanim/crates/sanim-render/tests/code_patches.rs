use sanim_core::{Fraction, Geometry, Viewport};
use sanim_lang::{Engine, MemorySources};
use sanim_render::{Options, Renderer};

fn prepare(animation: &str, before: &str, patch: &str) -> (Renderer, sanim_render::Scene, Options) {
    let mut files = MemorySources::default();
    files.insert("before.txt", before);
    files.insert("change.patch", patch);
    files.insert("main.sanim", format!("let code=CodeView(src=\"before.txt\",lang=\"plain\",font_size=20pt,line_numbers=false)\nadd(code)\nwait(1s)\ncode.patch(\"change.patch\",animation=\"{animation}\",duration=1s,ease=\"linear\")\nwait(1s)\n"));
    let prepared = Engine::new(Viewport {
        width: 360,
        height: 220,
        dpi: 72,
        ..Default::default()
    })
    .prepare("main.sanim", &files, &Default::default())
    .unwrap();
    let mut renderer = Renderer::default();
    let scene = renderer.prepare(prepared.animation).unwrap();
    let mut options = Options::for_animation(scene.animation());
    options.encode_png = false;
    (renderer, scene, options)
}

fn texts(elements: &[sanim_core::Element]) -> Vec<String> {
    fn visit(element: &sanim_core::Element, words: &mut Vec<String>) {
        if let Geometry::Text { text, .. } = element.geometry.as_ref() {
            words.push(text.clone());
        }
        if let Some(children) = element.geometry.children() {
            for child in children {
                visit(child, words);
            }
        }
    }
    let mut words = vec![];
    for element in elements {
        visit(element, &mut words);
    }
    words
}

#[test]
fn actual_patch_methods_render_distinct_fade_and_morph_but_identical_final_code() {
    let patch = "--- a/before.txt\n+++ b/before.txt\n@@ -1 +1,2 @@\n+\n alpha\n";
    let (mut morph, morph_scene, options) = prepare("morph", "alpha\n", patch);
    let (mut fade, fade_scene, _) = prepare("fade", "alpha\n", patch);
    let middle = Fraction::new(3u64, 2u64);
    let morph_middle = morph.render(&morph_scene, middle, &options).unwrap();
    let fade_middle = fade.render(&fade_scene, middle, &options).unwrap();
    assert!(
        morph_middle.rgba != fade_middle.rgba,
        "fade and morph must render different intermediate frames"
    );
    assert!(
        morph.render(&morph_scene, 2.into(), &options).unwrap().rgba
            == fade.render(&fade_scene, 2.into(), &options).unwrap().rgba,
        "presets must converge to identical target pixels"
    );
    let computed = fade.states_computed;
    assert!(fade.render(&fade_scene, middle, &options).unwrap().rgba == fade_middle.rgba);
    assert_eq!(fade.states_computed, computed);
}

#[test]
fn actual_typewriter_patch_exposes_complete_clusters_and_exact_final_pixels() {
    let patch = "--- a/before.txt\n+++ b/before.txt\n@@ -1 +1 @@\n-x\n+e\u{301}xyz\n";
    let (mut renderer, scene, options) = prepare("typewriter", "x\n", patch);
    // One deletion followed by four cursor clusters; two steps expose one new cluster.
    let time = Fraction::new(7u64, 5u64);
    let state = renderer.frame_state(&scene, time, &options).unwrap();
    assert_eq!(texts(state.elements()), ["e\u{301}"]);
    let final_typed = renderer.render(&scene, 2.into(), &options).unwrap();
    let (mut morph, morph_scene, _) = prepare("morph", "x\n", patch);
    assert!(
        final_typed.rgba == morph.render(&morph_scene, 2.into(), &options).unwrap().rgba,
        "typewriter must converge to exact native target pixels"
    );
    let reverse = renderer.frame_state(&scene, 0.into(), &options).unwrap();
    assert_eq!(texts(reverse.elements()), ["x"]);
}
